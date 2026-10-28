use std::fmt::Write;

use crate::db::{self, CATEGORIES, Comment, PRIORITIES, STATUSES, Ticket, User};

pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Texte multi-lignes échappé, retours à la ligne conservés.
fn text_block(s: &str) -> String {
    esc(s).replace('\n', "<br>")
}

const CSS: &str = r#"
:root { --bg:#f4f6f9; --card:#fff; --text:#1f2933; --muted:#6b7785; --primary:#2563eb;
        --border:#dde2e8; --danger:#dc2626; }
* { box-sizing:border-box; }
body { margin:0; font-family:system-ui,-apple-system,"Segoe UI",Roboto,sans-serif;
       background:var(--bg); color:var(--text); line-height:1.5; }
header { background:#1e293b; color:#fff; padding:0 16px; }
header .wrap { display:flex; align-items:center; gap:20px; height:56px; }
header a { color:#cbd5e1; text-decoration:none; }
header a:hover, header a.brand { color:#fff; }
header .brand { font-weight:700; font-size:1.1rem; }
header .spacer { flex:1; }
header form { margin:0; }
header button { background:none; border:1px solid #475569; color:#cbd5e1; padding:4px 10px; }
.wrap { max-width:1100px; margin:0 auto; }
main { padding:24px 16px; }
.card { background:var(--card); border:1px solid var(--border); border-radius:8px;
        padding:20px; margin-bottom:20px; }
h1 { font-size:1.5rem; margin:0 0 16px; }
h2 { font-size:1.15rem; margin:0 0 12px; }
label { display:block; font-weight:600; margin:12px 0 4px; font-size:.9rem; }
input[type=text], input[type=email], input[type=password], input[type=search], select, textarea {
    width:100%; padding:8px 10px; border:1px solid var(--border); border-radius:6px;
    font:inherit; background:#fff; }
textarea { min-height:140px; resize:vertical; }
button, .btn { display:inline-block; background:var(--primary); color:#fff; border:0;
    border-radius:6px; padding:8px 16px; font:inherit; cursor:pointer; text-decoration:none; }
button.secondary { background:#64748b; }
button.danger { background:var(--danger); }
.row { display:grid; grid-template-columns:repeat(auto-fit,minmax(200px,1fr)); gap:0 16px; }
.muted { color:var(--muted); font-size:.9rem; }
.flash { padding:12px 16px; border-radius:6px; margin-bottom:20px; background:#dcfce7;
         border:1px solid #86efac; }
.flash.error { background:#fee2e2; border-color:#fca5a5; }
table { width:100%; border-collapse:collapse; }
th, td { text-align:left; padding:8px 10px; border-bottom:1px solid var(--border); font-size:.92rem; }
th { color:var(--muted); font-weight:600; }
tr:hover td { background:#f8fafc; }
td a { color:var(--primary); text-decoration:none; font-weight:500; }
.badge { display:inline-block; padding:2px 8px; border-radius:999px; font-size:.78rem;
         font-weight:600; white-space:nowrap; }
.st-nouveau { background:#dbeafe; color:#1e40af; } .st-en_cours { background:#fef3c7; color:#92400e; }
.st-en_attente { background:#ede9fe; color:#5b21b6; } .st-resolu { background:#dcfce7; color:#166534; }
.st-ferme { background:#e5e7eb; color:#374151; }
.pr-basse { background:#f1f5f9; color:#475569; } .pr-normale { background:#e0f2fe; color:#075985; }
.pr-haute { background:#ffedd5; color:#9a3412; } .pr-critique { background:#fee2e2; color:#991b1b; }
.stats { display:flex; gap:12px; flex-wrap:wrap; margin-bottom:20px; }
.stat { background:var(--card); border:1px solid var(--border); border-radius:8px; padding:12px 18px;
        min-width:120px; text-decoration:none; color:inherit; }
.stat b { display:block; font-size:1.5rem; }
.filters { display:flex; gap:10px; flex-wrap:wrap; align-items:flex-end; }
.filters > * { flex:1 1 150px; }
.filters button { flex:0 0 auto; }
.comment { border-left:3px solid var(--border); padding:8px 14px; margin:12px 0; }
.comment.agent { border-color:var(--primary); }
.comment.internal { border-color:#f59e0b; background:#fffbeb; }
.comment .meta { font-size:.82rem; color:var(--muted); margin-bottom:4px; }
.grid2 { display:grid; grid-template-columns:2fr 1fr; gap:20px; }
@media (max-width:800px) { .grid2 { grid-template-columns:1fr; } .hide-sm { display:none; } }
dl { margin:0; } dt { font-weight:600; font-size:.85rem; color:var(--muted); margin-top:10px; }
dd { margin:0; }
code.link { display:block; padding:10px; background:#f1f5f9; border-radius:6px; word-break:break-all; }
"#;

pub fn layout(title: &str, user: Option<&User>, body: &str) -> String {
    let nav = match user {
        Some(u) => {
            let admin = if u.is_admin {
                r#"<a href="/agent/utilisateurs">Utilisateurs</a>"#
            } else {
                ""
            };
            format!(
                r#"<a href="/agent">Tickets</a>
                   <a href="/agent?assignes=moi">Mes tickets</a>
                   {admin}
                   <span class="spacer"></span>
                   <span class="muted" style="color:#94a3b8">{}</span>
                   <form method="post" action="/logout"><button>Déconnexion</button></form>"#,
                esc(&u.display_name)
            )
        }
        None => r#"<a href="/">Nouvelle demande</a>
                   <span class="spacer"></span>
                   <a href="/login">Espace technicien</a>"#
            .to_string(),
    };
    format!(
        r#"<!doctype html>
<html lang="fr"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{} – Support informatique</title><style>{CSS}</style></head>
<body><header><div class="wrap"><a class="brand" href="/">🛠️ Support IT</a>{nav}</div></header>
<main><div class="wrap">{body}</div></main></body></html>"#,
        esc(title)
    )
}

fn options(list: &[(&str, &str)], selected: &str) -> String {
    list.iter()
        .map(|(k, l)| {
            let sel = if *k == selected { " selected" } else { "" };
            format!(r#"<option value="{k}"{sel}>{}</option>"#, esc(l))
        })
        .collect()
}

pub fn status_badge(s: &str) -> String {
    format!(r#"<span class="badge st-{s}">{}</span>"#, esc(&db::label(STATUSES, s)))
}

pub fn priority_badge(p: &str) -> String {
    format!(r#"<span class="badge pr-{p}">{}</span>"#, esc(&db::label(PRIORITIES, p)))
}

fn flash(msg: Option<&str>, error: bool) -> String {
    match msg {
        Some(m) => format!(
            r#"<div class="flash{}">{}</div>"#,
            if error { " error" } else { "" },
            esc(m)
        ),
        None => String::new(),
    }
}

// ---------- Pages publiques ----------

pub fn new_ticket_page(error: Option<&str>) -> String {
    let body = format!(
        r#"{}
<div class="card">
<h1>Nouvelle demande informatique</h1>
<p class="muted">Décrivez votre problème ou votre besoin. Un technicien prendra en charge votre demande
et vous pourrez suivre son avancement grâce au lien fourni après l'envoi.</p>
<form method="post" action="/tickets">
  <div class="row">
    <div><label for="requester_name">Nom et prénom *</label>
      <input type="text" id="requester_name" name="requester_name" required maxlength="100"></div>
    <div><label for="requester_email">E-mail *</label>
      <input type="email" id="requester_email" name="requester_email" required maxlength="150"></div>
    <div><label for="requester_location">Service / Bureau</label>
      <input type="text" id="requester_location" name="requester_location" maxlength="100"></div>
  </div>
  <div class="row">
    <div><label for="category">Catégorie *</label>
      <select id="category" name="category" required>{}</select></div>
    <div><label for="priority">Urgence *</label>
      <select id="priority" name="priority" required>{}</select></div>
  </div>
  <label for="title">Objet *</label>
  <input type="text" id="title" name="title" required maxlength="200"
         placeholder="Ex. : Impossible d'imprimer depuis ce matin">
  <label for="description">Description détaillée *</label>
  <textarea id="description" name="description" required maxlength="10000"
    placeholder="Que se passe-t-il ? Depuis quand ? Message d'erreur éventuel, nom du poste…"></textarea>
  <p class="muted">Urgence « Critique » : réservée aux blocages complets empêchant plusieurs personnes
  de travailler.</p>
  <button type="submit">Envoyer la demande</button>
</form></div>"#,
        flash(error, true),
        options(CATEGORIES, ""),
        options(PRIORITIES, "normale"),
    );
    layout("Nouvelle demande", None, &body)
}

fn comments_html(comments: &[Comment]) -> String {
    if comments.is_empty() {
        return r#"<p class="muted">Aucun échange pour le moment.</p>"#.to_string();
    }
    comments
        .iter()
        .map(|c| {
            let class = if c.internal {
                "comment internal"
            } else if c.is_agent {
                "comment agent"
            } else {
                "comment"
            };
            let tag = if c.internal {
                " · <b>note interne</b>"
            } else if c.is_agent {
                " · support IT"
            } else {
                ""
            };
            format!(
                r#"<div class="{class}"><div class="meta"><b>{}</b>{tag} · {}</div>{}</div>"#,
                esc(&c.author),
                esc(&c.created_at),
                text_block(&c.body)
            )
        })
        .collect()
}

pub fn public_ticket_page(t: &Ticket, comments: &[Comment], base_url: &str, msg: Option<&str>) -> String {
    let closed = t.status == "ferme";
    let reply_form = if closed {
        r#"<p class="muted">Ce ticket est fermé. Pour un nouveau problème, <a href="/">créez une nouvelle demande</a>.</p>"#
            .to_string()
    } else {
        let mut f = format!(
            r#"<form method="post" action="/suivi/{tok}/commentaire">
  <label for="body">Ajouter un message</label>
  <textarea id="body" name="body" required maxlength="10000"></textarea>
  <button type="submit">Envoyer</button>
</form>"#,
            tok = t.token
        );
        if t.status == "resolu" {
            f.push_str(&format!(
                r#"<p class="muted" style="margin-top:16px">Le support a marqué ce ticket comme résolu.
Si le problème persiste, répondez ci-dessus : le ticket sera rouvert. Sinon vous pouvez le fermer.</p>
<form method="post" action="/suivi/{}/fermer"><button class="secondary">Confirmer et fermer le ticket</button></form>"#,
                t.token
            ));
        }
        f
    };
    let body = format!(
        r#"{flash}
<div class="card">
  <h1>Ticket #{id} – {title}</h1>
  <p>{status} {prio} <span class="muted">· {cat} · ouvert le {created}</span></p>
  <p class="muted">Lien de suivi (conservez-le) :</p>
  <code class="link">{base}/suivi/{tok}</code>
</div>
<div class="card"><h2>Description</h2>{desc}</div>
<div class="card"><h2>Échanges</h2>{comments}<hr style="border:0;border-top:1px solid var(--border);margin:20px 0">{reply_form}</div>"#,
        flash = flash(msg, false),
        id = t.id,
        title = esc(&t.title),
        status = status_badge(&t.status),
        prio = priority_badge(&t.priority),
        cat = esc(&db::label(CATEGORIES, &t.category)),
        created = esc(&t.created_at),
        base = esc(base_url),
        tok = t.token,
        desc = text_block(&t.description),
        comments = comments_html(comments),
    );
    layout(&format!("Ticket #{}", t.id), None, &body)
}

pub fn login_page(error: Option<&str>) -> String {
    let body = format!(
        r#"<div class="card" style="max-width:420px;margin:40px auto">
<h1>Espace technicien</h1>{}
<form method="post" action="/login">
  <label for="username">Identifiant</label>
  <input type="text" id="username" name="username" required autofocus autocomplete="username">
  <label for="password">Mot de passe</label>
  <input type="password" id="password" name="password" required autocomplete="current-password">
  <p><button type="submit">Se connecter</button></p>
</form></div>"#,
        flash(error, true)
    );
    layout("Connexion", None, &body)
}

// ---------- Pages techniciens ----------

pub struct DashboardParams<'a> {
    pub status: &'a str,
    pub priority: &'a str,
    pub assigned: &'a str,
    pub search: &'a str,
}

pub fn dashboard_page(
    user: &User,
    tickets: &[Ticket],
    counts: &[(String, i64)],
    users: &[User],
    p: &DashboardParams,
) -> String {
    let mut stats = String::new();
    for (k, l) in STATUSES {
        let n = counts.iter().find(|(s, _)| s == k).map(|(_, n)| *n).unwrap_or(0);
        let _ = write!(stats, r#"<a class="stat" href="/agent?statut={k}"><b>{n}</b>{}</a>"#, esc(l));
    }

    let mut status_opts = String::from(r#"<option value="">Tous (ouverts)</option>"#);
    status_opts.push_str(&format!(
        r#"<option value="tous"{}>Tous (y compris fermés)</option>"#,
        if p.status == "tous" { " selected" } else { "" }
    ));
    status_opts.push_str(&options(STATUSES, p.status));

    let mut assign_opts = String::from(r#"<option value="">Tout le monde</option>"#);
    for (v, l) in [("moi", "Moi"), ("aucun", "Non assignés")] {
        let sel = if p.assigned == v { " selected" } else { "" };
        let _ = write!(assign_opts, r#"<option value="{v}"{sel}>{l}</option>"#);
    }
    for u in users.iter().filter(|u| u.active) {
        let v = u.id.to_string();
        let sel = if p.assigned == v { " selected" } else { "" };
        let _ = write!(assign_opts, r#"<option value="{v}"{sel}>{}</option>"#, esc(&u.display_name));
    }

    let rows: String = if tickets.is_empty() {
        r#"<tr><td colspan="7" class="muted">Aucun ticket.</td></tr>"#.to_string()
    } else {
        tickets
            .iter()
            .map(|t| {
                format!(
                    r#"<tr><td>#{id}</td><td><a href="/agent/tickets/{id}">{title}</a></td>
<td>{status}</td><td>{prio}</td><td class="hide-sm">{req}</td>
<td class="hide-sm">{assignee}</td><td class="hide-sm muted">{updated}</td></tr>"#,
                    id = t.id,
                    title = esc(&t.title),
                    status = status_badge(&t.status),
                    prio = priority_badge(&t.priority),
                    req = esc(&t.requester_name),
                    assignee = t
                        .assignee_name
                        .as_deref()
                        .map(esc)
                        .unwrap_or_else(|| r#"<span class="muted">—</span>"#.into()),
                    updated = esc(&t.updated_at),
                )
            })
            .collect()
    };

    let body = format!(
        r#"<div class="stats">{stats}</div>
<div class="card">
<form class="filters" method="get" action="/agent">
  <div><label for="q">Recherche</label>
    <input type="search" id="q" name="q" value="{search}" placeholder="N°, objet, demandeur…"></div>
  <div><label for="statut">Statut</label><select id="statut" name="statut">{status_opts}</select></div>
  <div><label for="priorite">Priorité</label><select id="priorite" name="priorite">
    <option value="">Toutes</option>{prio_opts}</select></div>
  <div><label for="assignes">Assigné à</label><select id="assignes" name="assignes">{assign_opts}</select></div>
  <button type="submit">Filtrer</button>
</form></div>
<div class="card" style="padding:0;overflow-x:auto">
<table><thead><tr><th>N°</th><th>Objet</th><th>Statut</th><th>Priorité</th>
<th class="hide-sm">Demandeur</th><th class="hide-sm">Assigné</th><th class="hide-sm">Mis à jour</th></tr></thead>
<tbody>{rows}</tbody></table></div>
<p class="muted">{n} ticket(s) affiché(s).</p>"#,
        search = esc(p.search),
        prio_opts = options(PRIORITIES, p.priority),
        n = tickets.len(),
    );
    layout("Tickets", Some(user), &body)
}

pub fn agent_ticket_page(
    user: &User,
    t: &Ticket,
    comments: &[Comment],
    users: &[User],
    base_url: &str,
    msg: Option<&str>,
) -> String {
    let mut assign_opts = String::from(r#"<option value="">— Non assigné —</option>"#);
    for u in users.iter().filter(|u| u.active || Some(u.id) == t.assignee_id) {
        let sel = if Some(u.id) == t.assignee_id { " selected" } else { "" };
        let _ = write!(assign_opts, r#"<option value="{}"{sel}>{}</option>"#, u.id, esc(&u.display_name));
    }
    let take_btn = if t.assignee_id != Some(user.id) {
        format!(
            r#"<form method="post" action="/agent/tickets/{}/prendre" style="margin-top:10px">
<button class="secondary" type="submit">Me l'assigner</button></form>"#,
            t.id
        )
    } else {
        String::new()
    };

    let body = format!(
        r#"{flash}
<p><a href="/agent">← Retour à la liste</a></p>
<div class="grid2">
<div>
  <div class="card">
    <h1>#{id} – {title}</h1>
    <p>{status} {prio} <span class="muted">· {cat}</span></p>
    {desc}
  </div>
  <div class="card"><h2>Échanges</h2>{comments}
    <hr style="border:0;border-top:1px solid var(--border);margin:20px 0">
    <form method="post" action="/agent/tickets/{id}/commentaire">
      <label for="body">Répondre</label>
      <textarea id="body" name="body" required maxlength="10000"></textarea>
      <label style="font-weight:normal"><input type="checkbox" name="internal" value="1">
        Note interne (non visible par le demandeur)</label>
      <div class="row">
        <div><label for="new_status">Changer le statut (optionnel)</label>
        <select id="new_status" name="new_status"><option value="">— Inchangé —</option>{status_opts_plain}</select></div>
      </div>
      <p><button type="submit">Envoyer</button></p>
    </form>
  </div>
</div>
<div>
  <div class="card"><h2>Demandeur</h2><dl>
    <dt>Nom</dt><dd>{req}</dd>
    <dt>E-mail</dt><dd><a href="mailto:{email}">{email}</a></dd>
    <dt>Service / Bureau</dt><dd>{loc}</dd>
    <dt>Créé le</dt><dd>{created}</dd>
    <dt>Mis à jour</dt><dd>{updated}</dd>
    <dt>Lien de suivi demandeur</dt><dd><code class="link">{base}/suivi/{tok}</code></dd>
  </dl></div>
  <div class="card"><h2>Gestion</h2>
    <form method="post" action="/agent/tickets/{id}/modifier">
      <label for="status">Statut</label><select id="status" name="status">{status_opts}</select>
      <label for="priority">Priorité</label><select id="priority" name="priority">{prio_opts}</select>
      <label for="category">Catégorie</label><select id="category" name="category">{cat_opts}</select>
      <label for="assignee">Assigné à</label><select id="assignee" name="assignee">{assign_opts}</select>
      <p><button type="submit">Enregistrer</button></p>
    </form>
    {take_btn}
  </div>
</div>
</div>"#,
        flash = flash(msg, false),
        id = t.id,
        title = esc(&t.title),
        status = status_badge(&t.status),
        prio = priority_badge(&t.priority),
        cat = esc(&db::label(CATEGORIES, &t.category)),
        desc = text_block(&t.description),
        comments = comments_html(comments),
        status_opts_plain = options(STATUSES, ""),
        req = esc(&t.requester_name),
        email = esc(&t.requester_email),
        loc = if t.requester_location.is_empty() { "—".into() } else { esc(&t.requester_location) },
        created = esc(&t.created_at),
        updated = esc(&t.updated_at),
        base = esc(base_url),
        tok = t.token,
        status_opts = options(STATUSES, &t.status),
        prio_opts = options(PRIORITIES, &t.priority),
        cat_opts = options(CATEGORIES, &t.category),
    );
    layout(&format!("Ticket #{}", t.id), Some(user), &body)
}

pub fn users_page(user: &User, users: &[User], msg: Option<&str>, error: bool) -> String {
    let rows: String = users
        .iter()
        .map(|u| {
            let toggle = if u.id == user.id {
                String::new()
            } else {
                format!(
                    r#"<form method="post" action="/agent/utilisateurs/{}/actif" style="display:inline">
<button class="{}" type="submit">{}</button></form>"#,
                    u.id,
                    if u.active { "danger" } else { "secondary" },
                    if u.active { "Désactiver" } else { "Réactiver" }
                )
            };
            format!(
                r#"<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td>
<td><form method="post" action="/agent/utilisateurs/{}/mot-de-passe" style="display:flex;gap:6px">
<input type="password" name="password" placeholder="Nouveau mot de passe" required minlength="8">
<button class="secondary" type="submit">Changer</button></form></td><td>{toggle}</td></tr>"#,
                esc(&u.display_name),
                esc(&u.username),
                if u.is_admin { "Administrateur" } else { "Technicien" },
                if u.active { "Actif" } else { "Désactivé" },
                u.id,
            )
        })
        .collect();

    let body = format!(
        r#"{flash}
<div class="card" style="padding:0;overflow-x:auto"><table>
<thead><tr><th>Nom</th><th>Identifiant</th><th>Rôle</th><th>État</th><th>Mot de passe</th><th></th></tr></thead>
<tbody>{rows}</tbody></table></div>
<div class="card"><h2>Ajouter un technicien</h2>
<form method="post" action="/agent/utilisateurs">
  <div class="row">
    <div><label for="display_name">Nom affiché</label>
      <input type="text" id="display_name" name="display_name" required maxlength="100"></div>
    <div><label for="username">Identifiant</label>
      <input type="text" id="username" name="username" required maxlength="50"></div>
    <div><label for="password">Mot de passe (8 caractères min.)</label>
      <input type="password" id="password" name="password" required minlength="8"></div>
  </div>
  <label style="font-weight:normal"><input type="checkbox" name="is_admin" value="1"> Administrateur</label>
  <p><button type="submit">Créer le compte</button></p>
</form></div>"#,
        flash = flash(msg, error),
    );
    layout("Utilisateurs", Some(user), &body)
}
