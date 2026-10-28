mod auth;
mod db;
mod oidc;
mod views;

use std::sync::{Arc, Mutex};

use axum::{
    Form, Router,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{AppendHeaders, Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use rusqlite::Connection;
use serde::Deserialize;

use auth::{Admin, Agent, MaybeUser, Requester};
use db::{CATEGORIES, PRIORITIES, STATUSES};

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
    pub base_url: Arc<String>,
    /// Seules les adresses de ce domaine peuvent créer un compte demandeur.
    pub allowed_domain: Arc<String>,
    /// Connexion via Microsoft 365 / Google Workspace, si configurée.
    pub oidc: Option<Arc<oidc::Oidc>>,
}

impl AppState {
    fn sso_label(&self) -> Option<&'static str> {
        self.oidc.as_ref().map(|o| o.label())
    }
}

// ---------- Gestion des erreurs ----------

struct AppError(String);

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError(e.to_string())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        eprintln!("Erreur : {}", self.0);
        (StatusCode::INTERNAL_SERVER_ERROR, "Erreur interne du serveur").into_response()
    }
}

type AppResult<T> = Result<T, AppError>;

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, Html(views::layout("Introuvable", None,
        r#"<div class="card"><h1>Page introuvable</h1><p><a href="/">Retour à l'accueil</a></p></div>"#)))
        .into_response()
}

/// Messages de confirmation passés en paramètre `?msg=` après une redirection.
fn flash_message(code: Option<&str>) -> Option<&'static str> {
    match code? {
        "cree" => Some("Votre demande a bien été enregistrée. Vous la retrouverez dans « Mes demandes »."),
        "envoye" => Some("Message envoyé."),
        "maj" => Some("Ticket mis à jour."),
        "ferme" => Some("Ticket fermé. Merci !"),
        "user_cree" => Some("Compte créé."),
        "user_maj" => Some("Compte mis à jour."),
        _ => None,
    }
}

#[derive(Deserialize)]
struct MsgQuery {
    msg: Option<String>,
}

fn clean(s: &str, max: usize) -> String {
    s.trim().chars().take(max).collect()
}

// ---------- Côté demandeur ----------

async fn home(State(st): State<AppState>, MaybeUser(user): MaybeUser) -> Response {
    match user {
        None => Html(views::home_page(&st.allowed_domain, st.sso_label())).into_response(),
        Some(u) if u.is_agent() => Redirect::to("/agent").into_response(),
        Some(u) => Html(views::new_ticket_page(&u, None)).into_response(),
    }
}

#[derive(Deserialize)]
struct NewTicketForm {
    #[serde(default)]
    requester_location: String,
    category: String,
    priority: String,
    title: String,
    description: String,
}

async fn create_ticket(
    State(st): State<AppState>,
    Requester(user): Requester,
    Form(f): Form<NewTicketForm>,
) -> AppResult<Response> {
    let t = db::NewTicket {
        title: clean(&f.title, 200),
        description: clean(&f.description, 10_000),
        category: f.category,
        priority: f.priority,
        requester_name: user.display_name.clone(),
        requester_email: user.username.clone(),
        requester_location: clean(&f.requester_location, 100),
        requester_id: Some(user.id),
    };
    let error = if t.title.is_empty() || t.description.is_empty() {
        Some("Merci de remplir tous les champs obligatoires.")
    } else if !db::is_valid(CATEGORIES, &t.category) || !db::is_valid(PRIORITIES, &t.priority) {
        Some("Catégorie ou urgence invalide.")
    } else {
        None
    };
    if let Some(e) = error {
        return Ok((StatusCode::BAD_REQUEST, Html(views::new_ticket_page(&user, Some(e)))).into_response());
    }

    let token = auth::random_token();
    let id = db::create_ticket(&st.db.lock().unwrap(), &t, &token)?;
    println!("Nouveau ticket #{id} : {} ({})", t.title, t.requester_email);
    Ok(Redirect::to(&format!("/suivi/{token}?msg=cree")).into_response())
}

async fn my_tickets(State(st): State<AppState>, Requester(user): Requester) -> AppResult<Html<String>> {
    let tickets = db::tickets_for_requester(&st.db.lock().unwrap(), user.id)?;
    Ok(Html(views::my_tickets_page(&user, &tickets)))
}

// ---------- Inscription des demandeurs ----------

/// Vérifie que l'adresse est de la forme `quelquechose@<domaine autorisé>`.
fn email_allowed(email: &str, domain: &str) -> bool {
    match email.rsplit_once('@') {
        Some((local, d)) => {
            !local.is_empty()
                && !local.contains('@')
                && !email.contains(char::is_whitespace)
                && d.eq_ignore_ascii_case(domain)
        }
        None => false,
    }
}

async fn register_form(State(st): State<AppState>, MaybeUser(user): MaybeUser) -> Response {
    // Avec un fournisseur d'identité, le compte est créé à la première connexion.
    if user.is_some() || st.oidc.is_some() {
        return Redirect::to("/").into_response();
    }
    let empty = views::RegisterValues { display_name: "", email: "", location: "" };
    Html(views::register_page(&st.allowed_domain, &empty, None)).into_response()
}

#[derive(Deserialize)]
struct RegisterForm {
    display_name: String,
    email: String,
    #[serde(default)]
    location: String,
    password: String,
    password2: String,
}

async fn register(State(st): State<AppState>, Form(f): Form<RegisterForm>) -> AppResult<Response> {
    if st.oidc.is_some() {
        return Ok(Redirect::to("/login").into_response());
    }
    let display_name = clean(&f.display_name, 100);
    let email = clean(&f.email, 150).to_lowercase();
    let location = clean(&f.location, 100);
    let domain = st.allowed_domain.as_str();

    let error = if display_name.is_empty() {
        Some("Merci d'indiquer votre nom.".to_string())
    } else if !email_allowed(&email, domain) {
        Some(format!("Seules les adresses @{domain} peuvent créer un compte."))
    } else if f.password.chars().count() < 8 {
        Some("Le mot de passe doit contenir au moins 8 caractères.".into())
    } else if f.password != f.password2 {
        Some("Les deux mots de passe ne correspondent pas.".into())
    } else if db::username_exists(&st.db.lock().unwrap(), &email)? {
        Some("Un compte existe déjà avec cette adresse. Connectez-vous.".into())
    } else {
        None
    };
    if let Some(e) = error {
        let values = views::RegisterValues { display_name: &display_name, email: &email, location: &location };
        let page = views::register_page(domain, &values, Some(&e));
        return Ok((StatusCode::BAD_REQUEST, Html(page)).into_response());
    }

    let hash = auth::hash_password(&f.password);
    let token = auth::random_token();
    {
        let conn = st.db.lock().unwrap();
        let id = db::create_requester(&conn, &email, &display_name, &location, &hash)?;
        db::link_tickets_by_email(&conn, id, &email)?;
        db::create_session(&conn, &token, id)?;
    }
    println!("Nouveau compte demandeur : {email}");
    Ok(([(header::SET_COOKIE, auth::session_cookie(&token))], Redirect::to("/mes-demandes")).into_response())
}

async fn public_ticket(
    State(st): State<AppState>,
    MaybeUser(viewer): MaybeUser,
    Path(token): Path<String>,
    Query(q): Query<MsgQuery>,
) -> AppResult<Response> {
    let conn = st.db.lock().unwrap();
    let Some(t) = db::ticket_by_token(&conn, &token)? else {
        return Ok(not_found());
    };
    let comments = db::comments(&conn, t.id, false)?;
    let msg = flash_message(q.msg.as_deref());
    let page = views::public_ticket_page(viewer.as_ref(), &t, &comments, &st.base_url, msg);
    Ok(Html(page).into_response())
}

#[derive(Deserialize)]
struct CommentForm {
    body: String,
    internal: Option<String>,
    new_status: Option<String>,
}

async fn public_comment(
    State(st): State<AppState>,
    Path(token): Path<String>,
    Form(f): Form<CommentForm>,
) -> AppResult<Response> {
    let conn = st.db.lock().unwrap();
    let Some(t) = db::ticket_by_token(&conn, &token)? else {
        return Ok(not_found());
    };
    let body = clean(&f.body, 10_000);
    if !body.is_empty() && t.status != "ferme" {
        db::add_comment(&conn, t.id, &t.requester_name, false, false, &body)?;
        // Une réponse du demandeur relance le ticket.
        if t.status == "resolu" || t.status == "en_attente" {
            db::set_status(&conn, t.id, "en_cours")?;
        }
    }
    Ok(Redirect::to(&format!("/suivi/{token}?msg=envoye")).into_response())
}

async fn public_close(State(st): State<AppState>, Path(token): Path<String>) -> AppResult<Response> {
    let conn = st.db.lock().unwrap();
    let Some(t) = db::ticket_by_token(&conn, &token)? else {
        return Ok(not_found());
    };
    if t.status == "resolu" {
        db::set_status(&conn, t.id, "ferme")?;
        db::add_comment(&conn, t.id, &t.requester_name, false, false, "Ticket fermé par le demandeur.")?;
    }
    Ok(Redirect::to(&format!("/suivi/{token}?msg=ferme")).into_response())
}

// ---------- Authentification ----------

fn home_for(user: &db::User) -> &'static str {
    if user.is_agent() { "/agent" } else { "/mes-demandes" }
}

async fn login_form(State(st): State<AppState>, MaybeUser(user): MaybeUser) -> Response {
    match user {
        Some(u) => Redirect::to(home_for(&u)).into_response(),
        None => Html(views::login_page(None, st.sso_label())).into_response(),
    }
}

#[derive(Deserialize)]
struct LoginForm {
    username: String,
    password: String,
}

async fn login(State(st): State<AppState>, Form(f): Form<LoginForm>) -> AppResult<Response> {
    let creds = db::user_credentials(&st.db.lock().unwrap(), f.username.trim())?;
    let user_id = creds.and_then(|(id, hash)| auth::verify_password(&f.password, &hash).then_some(id));
    let Some(user_id) = user_id else {
        let page = views::login_page(Some("Identifiant ou mot de passe incorrect."), st.sso_label());
        return Ok((StatusCode::UNAUTHORIZED, Html(page)).into_response());
    };
    let role = db::user_role(&st.db.lock().unwrap(), user_id)?;
    // Avec un fournisseur d'identité, les demandeurs passent obligatoirement par lui.
    if let Some(label) = st.sso_label().filter(|_| role == db::ROLE_REQUESTER) {
        let msg = format!("Utilisez le bouton « {label} ».");
        return Ok((StatusCode::UNAUTHORIZED, Html(views::login_page(Some(&msg), Some(label)))).into_response());
    }
    let token = auth::random_token();
    db::create_session(&st.db.lock().unwrap(), &token, user_id)?;
    let target = if role == db::ROLE_REQUESTER { "/mes-demandes" } else { "/agent" };
    Ok(([(header::SET_COOKIE, auth::session_cookie(&token))], Redirect::to(target)).into_response())
}

async fn logout(State(st): State<AppState>, req: axum::extract::Request) -> AppResult<Response> {
    let (parts, _) = req.into_parts();
    if let Some(token) = auth::cookie_token(&parts) {
        db::delete_session(&st.db.lock().unwrap(), &token)?;
    }
    Ok(([(header::SET_COOKIE, auth::clear_cookie())], Redirect::to("/login")).into_response())
}

// ---------- Connexion via Microsoft 365 / Google Workspace ----------

async fn sso_start(State(st): State<AppState>) -> AppResult<Response> {
    let Some(oidc) = st.oidc.clone() else {
        return Ok(not_found());
    };
    let (state, nonce, verifier) = (auth::random_token(), auth::random_token(), auth::random_token());
    db::save_oauth_state(&st.db.lock().unwrap(), &state, &verifier, &nonce)?;
    let url = oidc.authorize_url(&state, &nonce, &verifier);
    Ok(([(header::SET_COOKIE, auth::state_cookie(&state))], Redirect::to(&url)).into_response())
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

fn sso_error(st: &AppState, msg: &str) -> Response {
    let page = views::login_page(Some(msg), st.sso_label());
    ([(header::SET_COOKIE, auth::clear_state_cookie())], (StatusCode::UNAUTHORIZED, Html(page))).into_response()
}

async fn sso_callback(
    State(st): State<AppState>,
    Query(q): Query<CallbackQuery>,
    req: axum::extract::Request,
) -> AppResult<Response> {
    let Some(oidc) = st.oidc.clone() else {
        return Ok(not_found());
    };
    if let Some(err) = q.error {
        eprintln!("Connexion refusée par le fournisseur : {err} {}", q.error_description.unwrap_or_default());
        return Ok(sso_error(&st, "Connexion annulée ou refusée."));
    }
    let (parts, _) = req.into_parts();
    let (Some(code), Some(state)) = (q.code, q.state) else {
        return Ok(sso_error(&st, "Réponse de connexion incomplète."));
    };
    // L'état doit correspondre au cookie posé par ce navigateur (protection CSRF).
    if auth::cookie_value(&parts, auth::STATE_COOKIE).as_deref() != Some(state.as_str()) {
        return Ok(sso_error(&st, "Session de connexion expirée, veuillez réessayer."));
    }
    let Some((verifier, nonce)) = db::take_oauth_state(&st.db.lock().unwrap(), &state)? else {
        return Ok(sso_error(&st, "Session de connexion expirée, veuillez réessayer."));
    };

    let identity = match oidc.exchange(&code, &verifier, &nonce).await {
        Ok(i) => i,
        Err(e) => {
            eprintln!("Échec de connexion via le fournisseur : {e}");
            let msg = if e.starts_with("seules les adresses") || e.starts_with("compte Google") {
                format!("Accès réservé aux comptes @{}.", st.allowed_domain)
            } else {
                "La connexion a échoué. Réessayez ou contactez le support informatique.".to_string()
            };
            return Ok(sso_error(&st, &msg));
        }
    };

    let token = auth::random_token();
    let target = {
        let conn = st.db.lock().unwrap();
        let user_id = match db::user_by_username(&conn, &identity.email)? {
            Some(u) if !u.active => {
                drop(conn);
                return Ok(sso_error(&st, "Votre compte a été désactivé. Contactez le support informatique."));
            }
            Some(u) => u.id,
            None => {
                // Première connexion : création du compte demandeur, sans mot de passe local.
                let id = db::create_requester(&conn, &identity.email, &identity.name, "", "!sso")?;
                db::link_tickets_by_email(&conn, id, &identity.email)?;
                println!("Nouveau compte demandeur (fournisseur d'identité) : {}", identity.email);
                id
            }
        };
        db::create_session(&conn, &token, user_id)?;
        if db::user_role(&conn, user_id)? == db::ROLE_REQUESTER { "/mes-demandes" } else { "/agent" }
    };
    // AppendHeaders : deux en-têtes Set-Cookie (un tableau simple écraserait le premier).
    let cookies = AppendHeaders([
        (header::SET_COOKIE, auth::session_cookie(&token)),
        (header::SET_COOKIE, auth::clear_state_cookie()),
    ]);
    Ok((cookies, Redirect::to(target)).into_response())
}

// ---------- Côté technicien ----------

#[derive(Deserialize, Default)]
struct DashboardQuery {
    #[serde(default)]
    statut: String,
    #[serde(default)]
    priorite: String,
    #[serde(default)]
    assignes: String,
    #[serde(default)]
    q: String,
}

async fn dashboard(
    State(st): State<AppState>,
    Agent(user): Agent,
    Query(q): Query<DashboardQuery>,
) -> AppResult<Html<String>> {
    let filter = db::TicketFilter {
        status: db::is_valid(STATUSES, &q.statut).then(|| q.statut.clone()),
        open_only: q.statut.is_empty(),
        priority: db::is_valid(PRIORITIES, &q.priorite).then(|| q.priorite.clone()),
        unassigned: q.assignes == "aucun",
        assignee: match q.assignes.as_str() {
            "moi" => Some(user.id),
            s => s.parse().ok(),
        },
        search: Some(q.q.trim().to_string()).filter(|s| !s.is_empty()),
    };
    let conn = st.db.lock().unwrap();
    let tickets = db::list_tickets(&conn, &filter)?;
    let counts = db::status_counts(&conn)?;
    let users = db::list_users(&conn)?;
    let params = views::DashboardParams {
        status: &q.statut,
        priority: &q.priorite,
        assigned: &q.assignes,
        search: &q.q,
    };
    Ok(Html(views::dashboard_page(&user, &tickets, &counts, &users, &params)))
}

async fn agent_ticket(
    State(st): State<AppState>,
    Agent(user): Agent,
    Path(id): Path<i64>,
    Query(q): Query<MsgQuery>,
) -> AppResult<Response> {
    let conn = st.db.lock().unwrap();
    let Some(t) = db::ticket_by_id(&conn, id)? else {
        return Ok(not_found());
    };
    let comments = db::comments(&conn, id, true)?;
    let users = db::list_users(&conn)?;
    let msg = flash_message(q.msg.as_deref());
    Ok(Html(views::agent_ticket_page(&user, &t, &comments, &users, &st.base_url, msg)).into_response())
}

#[derive(Deserialize)]
struct UpdateForm {
    status: String,
    priority: String,
    category: String,
    #[serde(default)]
    assignee: String,
}

async fn agent_update(
    State(st): State<AppState>,
    Agent(user): Agent,
    Path(id): Path<i64>,
    Form(f): Form<UpdateForm>,
) -> AppResult<Response> {
    if !db::is_valid(STATUSES, &f.status)
        || !db::is_valid(PRIORITIES, &f.priority)
        || !db::is_valid(CATEGORIES, &f.category)
    {
        return Ok((StatusCode::BAD_REQUEST, "Valeur invalide").into_response());
    }
    let conn = st.db.lock().unwrap();
    let Some(t) = db::ticket_by_id(&conn, id)? else {
        return Ok(not_found());
    };
    let assignee: Option<i64> = f.assignee.parse().ok();
    db::update_ticket(&conn, id, &f.status, &f.priority, &f.category, assignee)?;

    // Historique des changements en note interne.
    let mut changes = Vec::new();
    if t.status != f.status {
        changes.push(format!(
            "Statut : {} → {}",
            db::label(STATUSES, &t.status),
            db::label(STATUSES, &f.status)
        ));
    }
    if t.priority != f.priority {
        changes.push(format!(
            "Priorité : {} → {}",
            db::label(PRIORITIES, &t.priority),
            db::label(PRIORITIES, &f.priority)
        ));
    }
    if t.category != f.category {
        changes.push(format!("Catégorie : {}", db::label(CATEGORIES, &f.category)));
    }
    if t.assignee_id != assignee {
        let name = db::list_users(&conn)?
            .into_iter()
            .find(|u| Some(u.id) == assignee)
            .map(|u| u.display_name)
            .unwrap_or_else(|| "personne".into());
        changes.push(format!("Assigné à : {name}"));
    }
    if !changes.is_empty() {
        db::add_comment(&conn, id, &user.display_name, true, true, &changes.join("\n"))?;
    }
    Ok(Redirect::to(&format!("/agent/tickets/{id}?msg=maj")).into_response())
}

async fn agent_take(
    State(st): State<AppState>,
    Agent(user): Agent,
    Path(id): Path<i64>,
) -> AppResult<Response> {
    let conn = st.db.lock().unwrap();
    let Some(t) = db::ticket_by_id(&conn, id)? else {
        return Ok(not_found());
    };
    let status = if t.status == "nouveau" { "en_cours" } else { t.status.as_str() };
    db::update_ticket(&conn, id, status, &t.priority, &t.category, Some(user.id))?;
    db::add_comment(&conn, id, &user.display_name, true, true, "Ticket pris en charge.")?;
    Ok(Redirect::to(&format!("/agent/tickets/{id}?msg=maj")).into_response())
}

async fn agent_comment(
    State(st): State<AppState>,
    Agent(user): Agent,
    Path(id): Path<i64>,
    Form(f): Form<CommentForm>,
) -> AppResult<Response> {
    let conn = st.db.lock().unwrap();
    let Some(t) = db::ticket_by_id(&conn, id)? else {
        return Ok(not_found());
    };
    let body = clean(&f.body, 10_000);
    let internal = f.internal.is_some();
    if !body.is_empty() {
        db::add_comment(&conn, id, &user.display_name, true, internal, &body)?;
    }
    let new_status = f.new_status.filter(|s| db::is_valid(STATUSES, s));
    match new_status {
        Some(s) => db::set_status(&conn, id, &s)?,
        // Première réponse publique : le ticket passe « En cours ».
        None if !internal && t.status == "nouveau" => db::set_status(&conn, id, "en_cours")?,
        None => {}
    }
    Ok(Redirect::to(&format!("/agent/tickets/{id}?msg=envoye")).into_response())
}

// ---------- Administration des comptes ----------

async fn users_list(
    State(st): State<AppState>,
    Admin(user): Admin,
    Query(q): Query<MsgQuery>,
) -> AppResult<Html<String>> {
    let users = db::list_users(&st.db.lock().unwrap())?;
    let msg = flash_message(q.msg.as_deref());
    Ok(Html(views::users_page(&user, &users, msg, false)))
}

#[derive(Deserialize)]
struct NewUserForm {
    display_name: String,
    username: String,
    password: String,
    is_admin: Option<String>,
}

async fn users_create(
    State(st): State<AppState>,
    Admin(user): Admin,
    Form(f): Form<NewUserForm>,
) -> AppResult<Response> {
    let username = clean(&f.username, 50);
    let display_name = clean(&f.display_name, 100);
    let error = if username.is_empty() || display_name.is_empty() {
        Some("Nom et identifiant obligatoires.")
    } else if f.password.chars().count() < 8 {
        Some("Le mot de passe doit contenir au moins 8 caractères.")
    } else {
        None
    };
    let error = match error {
        Some(e) => Some(e),
        None => {
            let hash = auth::hash_password(&f.password);
            let conn = st.db.lock().unwrap();
            match db::create_agent(&conn, &username, &display_name, &hash, f.is_admin.is_some()) {
                Ok(_) => None,
                Err(rusqlite::Error::SqliteFailure(e, _))
                    if e.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    Some("Cet identifiant existe déjà.")
                }
                Err(e) => return Err(e.into()),
            }
        }
    };
    match error {
        None => Ok(Redirect::to("/agent/utilisateurs?msg=user_cree").into_response()),
        Some(e) => {
            let users = db::list_users(&st.db.lock().unwrap())?;
            Ok((StatusCode::BAD_REQUEST, Html(views::users_page(&user, &users, Some(e), true))).into_response())
        }
    }
}

async fn users_toggle(
    State(st): State<AppState>,
    Admin(admin): Admin,
    Path(id): Path<i64>,
) -> AppResult<Response> {
    if id != admin.id {
        let conn = st.db.lock().unwrap();
        if let Some(u) = db::list_users(&conn)?.into_iter().find(|u| u.id == id) {
            db::set_user_active(&conn, id, !u.active)?;
        }
    }
    Ok(Redirect::to("/agent/utilisateurs?msg=user_maj").into_response())
}

#[derive(Deserialize)]
struct PasswordForm {
    password: String,
}

async fn users_password(
    State(st): State<AppState>,
    Admin(_): Admin,
    Path(id): Path<i64>,
    Form(f): Form<PasswordForm>,
) -> AppResult<Response> {
    if f.password.chars().count() < 8 {
        return Ok((StatusCode::BAD_REQUEST, "Mot de passe trop court (8 caractères minimum)").into_response());
    }
    let hash = auth::hash_password(&f.password);
    db::set_user_password(&st.db.lock().unwrap(), id, &hash)?;
    Ok(Redirect::to("/agent/utilisateurs?msg=user_maj").into_response())
}

// ---------- Démarrage ----------

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Crée un compte administrateur au premier lancement.
fn bootstrap_admin(conn: &Connection) -> rusqlite::Result<()> {
    if db::count_users(conn)? > 0 {
        return Ok(());
    }
    let (password, generated) = match std::env::var("ADMIN_PASSWORD") {
        Ok(p) if !p.is_empty() => (p, false),
        _ => (auth::random_token()[..16].to_string(), true),
    };
    db::create_agent(conn, "admin", "Administrateur", &auth::hash_password(&password), true)?;
    println!("Compte administrateur créé : identifiant « admin »");
    if generated {
        println!("Mot de passe généré : {password}  (changez-le dans « Comptes »)");
    }
    Ok(())
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/", get(home))
        .route("/tickets", post(create_ticket))
        .route("/mes-demandes", get(my_tickets))
        .route("/inscription", get(register_form).post(register))
        .route("/suivi/{token}", get(public_ticket))
        .route("/suivi/{token}/commentaire", post(public_comment))
        .route("/suivi/{token}/fermer", post(public_close))
        .route("/login", get(login_form).post(login))
        .route("/auth/login", get(sso_start))
        .route("/auth/callback", get(sso_callback))
        .route("/logout", post(logout))
        .route("/agent", get(dashboard))
        .route("/agent/tickets/{id}", get(agent_ticket))
        .route("/agent/tickets/{id}/modifier", post(agent_update))
        .route("/agent/tickets/{id}/prendre", post(agent_take))
        .route("/agent/tickets/{id}/commentaire", post(agent_comment))
        .route("/agent/utilisateurs", get(users_list).post(users_create))
        .route("/agent/utilisateurs/{id}/actif", post(users_toggle))
        .route("/agent/utilisateurs/{id}/mot-de-passe", post(users_password))
        .fallback(|| async { not_found() })
        .with_state(state)
}

#[tokio::main]
async fn main() {
    let db_path = env_or("TICKETING_DB", "tickets.db");
    let addr = env_or("TICKETING_ADDR", "0.0.0.0:8080");
    let base_url = env_or("TICKETING_BASE_URL", "http://localhost:8080");
    let allowed_domain = env_or("TICKETING_ALLOWED_DOMAIN", "sias.fr").trim().trim_start_matches('@').to_lowercase();

    let conn = db::open(&db_path).expect("ouverture de la base de données impossible");
    bootstrap_admin(&conn).expect("création du compte administrateur impossible");

    let base_url = base_url.trim_end_matches('/').to_string();
    let oidc = oidc::Oidc::from_env(&base_url, &allowed_domain)
        .await
        .unwrap_or_else(|e| panic!("configuration de la connexion par fournisseur invalide : {e}"));
    match &oidc {
        Some(o) => println!("Connexion par fournisseur d'identité active (URL de retour : {})", o.redirect_uri()),
        None => println!("Connexion par fournisseur d'identité désactivée (inscription par mot de passe)"),
    }

    let state = AppState {
        db: Arc::new(Mutex::new(conn)),
        base_url: Arc::new(base_url),
        allowed_domain: Arc::new(allowed_domain),
        oidc: oidc.map(Arc::new),
    };

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("impossible d'écouter sur {addr} : {e}"));
    println!("Support IT démarré sur http://{addr} (base : {db_path}, inscriptions : @{})", state.allowed_domain);
    axum::serve(listener, app(state)).await.unwrap();
}
