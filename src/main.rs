mod auth;
mod db;
mod views;

use std::sync::{Arc, Mutex};

use axum::{
    Form, Router,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use rusqlite::Connection;
use serde::Deserialize;

use auth::{Admin, Agent};
use db::{CATEGORIES, PRIORITIES, STATUSES};

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
    pub base_url: Arc<String>,
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
        "cree" => Some("Votre demande a bien été enregistrée. Conservez le lien de suivi ci-dessous."),
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

async fn new_ticket_form() -> Html<String> {
    Html(views::new_ticket_page(None))
}

#[derive(Deserialize)]
struct NewTicketForm {
    requester_name: String,
    requester_email: String,
    #[serde(default)]
    requester_location: String,
    category: String,
    priority: String,
    title: String,
    description: String,
}

async fn create_ticket(State(st): State<AppState>, Form(f): Form<NewTicketForm>) -> AppResult<Response> {
    let t = db::NewTicket {
        title: clean(&f.title, 200),
        description: clean(&f.description, 10_000),
        category: f.category,
        priority: f.priority,
        requester_name: clean(&f.requester_name, 100),
        requester_email: clean(&f.requester_email, 150),
        requester_location: clean(&f.requester_location, 100),
    };
    let error = if t.title.is_empty() || t.description.is_empty() || t.requester_name.is_empty() {
        Some("Merci de remplir tous les champs obligatoires.")
    } else if !t.requester_email.contains('@') {
        Some("Adresse e-mail invalide.")
    } else if !db::is_valid(CATEGORIES, &t.category) || !db::is_valid(PRIORITIES, &t.priority) {
        Some("Catégorie ou urgence invalide.")
    } else {
        None
    };
    if let Some(e) = error {
        return Ok((StatusCode::BAD_REQUEST, Html(views::new_ticket_page(Some(e)))).into_response());
    }

    let token = auth::random_token();
    let id = db::create_ticket(&st.db.lock().unwrap(), &t, &token)?;
    println!("Nouveau ticket #{id} : {} ({})", t.title, t.requester_email);
    Ok(Redirect::to(&format!("/suivi/{token}?msg=cree")).into_response())
}

async fn public_ticket(
    State(st): State<AppState>,
    Path(token): Path<String>,
    Query(q): Query<MsgQuery>,
) -> AppResult<Response> {
    let conn = st.db.lock().unwrap();
    let Some(t) = db::ticket_by_token(&conn, &token)? else {
        return Ok(not_found());
    };
    let comments = db::comments(&conn, t.id, false)?;
    let msg = flash_message(q.msg.as_deref());
    Ok(Html(views::public_ticket_page(&t, &comments, &st.base_url, msg)).into_response())
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

async fn login_form() -> Html<String> {
    Html(views::login_page(None))
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
        let page = views::login_page(Some("Identifiant ou mot de passe incorrect."));
        return Ok((StatusCode::UNAUTHORIZED, Html(page)).into_response());
    };
    let token = auth::random_token();
    db::create_session(&st.db.lock().unwrap(), &token, user_id)?;
    Ok(([(header::SET_COOKIE, auth::session_cookie(&token))], Redirect::to("/agent")).into_response())
}

async fn logout(State(st): State<AppState>, req: axum::extract::Request) -> AppResult<Response> {
    let (parts, _) = req.into_parts();
    if let Some(token) = auth::cookie_token(&parts) {
        db::delete_session(&st.db.lock().unwrap(), &token)?;
    }
    Ok(([(header::SET_COOKIE, auth::clear_cookie())], Redirect::to("/login")).into_response())
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
            match db::create_user(&conn, &username, &display_name, &hash, f.is_admin.is_some()) {
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
    db::create_user(conn, "admin", "Administrateur", &auth::hash_password(&password), true)?;
    println!("Compte administrateur créé : identifiant « admin »");
    if generated {
        println!("Mot de passe généré : {password}  (changez-le dans « Utilisateurs »)");
    }
    Ok(())
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/", get(new_ticket_form))
        .route("/tickets", post(create_ticket))
        .route("/suivi/{token}", get(public_ticket))
        .route("/suivi/{token}/commentaire", post(public_comment))
        .route("/suivi/{token}/fermer", post(public_close))
        .route("/login", get(login_form).post(login))
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

    let conn = db::open(&db_path).expect("ouverture de la base de données impossible");
    bootstrap_admin(&conn).expect("création du compte administrateur impossible");

    let state = AppState {
        db: Arc::new(Mutex::new(conn)),
        base_url: Arc::new(base_url.trim_end_matches('/').to_string()),
    };

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("impossible d'écouter sur {addr} : {e}"));
    println!("Support IT démarré sur http://{addr} (base : {db_path})");
    axum::serve(listener, app(state)).await.unwrap();
}
