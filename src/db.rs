use rusqlite::{Connection, OptionalExtension, Result, params};

pub const STATUSES: &[(&str, &str)] = &[
    ("nouveau", "Nouveau"),
    ("en_cours", "En cours"),
    ("en_attente", "En attente"),
    ("resolu", "Résolu"),
    ("ferme", "Fermé"),
];

pub const PRIORITIES: &[(&str, &str)] = &[
    ("basse", "Basse"),
    ("normale", "Normale"),
    ("haute", "Haute"),
    ("critique", "Critique"),
];

pub const CATEGORIES: &[(&str, &str)] = &[
    ("materiel", "Matériel (PC, écran, périphérique)"),
    ("logiciel", "Logiciel / Application"),
    ("reseau", "Réseau / Internet / VPN"),
    ("compte", "Compte / Accès / Mot de passe"),
    ("messagerie", "Messagerie"),
    ("impression", "Imprimante / Scanner"),
    ("telephonie", "Téléphonie"),
    ("autre", "Autre"),
];

/// Retourne le libellé d'une clé, ou la clé elle-même si inconnue.
pub fn label(list: &[(&'static str, &'static str)], key: &str) -> String {
    list.iter()
        .find(|(k, _)| *k == key)
        .map(|(_, l)| l.to_string())
        .unwrap_or_else(|| key.to_string())
}

pub fn is_valid(list: &[(&str, &str)], key: &str) -> bool {
    list.iter().any(|(k, _)| *k == key)
}

pub struct Ticket {
    pub id: i64,
    pub token: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub priority: String,
    pub status: String,
    pub requester_name: String,
    pub requester_email: String,
    pub requester_location: String,
    pub assignee_id: Option<i64>,
    pub assignee_name: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub struct Comment {
    pub author: String,
    pub is_agent: bool,
    pub internal: bool,
    pub body: String,
    pub created_at: String,
}

pub struct User {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub is_admin: bool,
    pub active: bool,
}

pub struct NewTicket {
    pub title: String,
    pub description: String,
    pub category: String,
    pub priority: String,
    pub requester_name: String,
    pub requester_email: String,
    pub requester_location: String,
}

#[derive(Default)]
pub struct TicketFilter {
    pub status: Option<String>,
    pub priority: Option<String>,
    pub assignee: Option<i64>,
    pub unassigned: bool,
    pub open_only: bool,
    pub search: Option<String>,
}

pub fn open(path: &str) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;

         CREATE TABLE IF NOT EXISTS users (
             id            INTEGER PRIMARY KEY,
             username      TEXT NOT NULL UNIQUE,
             display_name  TEXT NOT NULL,
             password_hash TEXT NOT NULL,
             is_admin      INTEGER NOT NULL DEFAULT 0,
             active        INTEGER NOT NULL DEFAULT 1
         );

         CREATE TABLE IF NOT EXISTS sessions (
             token      TEXT PRIMARY KEY,
             user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
             expires_at TEXT NOT NULL
         );

         CREATE TABLE IF NOT EXISTS tickets (
             id                 INTEGER PRIMARY KEY,
             token              TEXT NOT NULL UNIQUE,
             title              TEXT NOT NULL,
             description        TEXT NOT NULL,
             category           TEXT NOT NULL,
             priority           TEXT NOT NULL,
             status             TEXT NOT NULL DEFAULT 'nouveau',
             requester_name     TEXT NOT NULL,
             requester_email    TEXT NOT NULL,
             requester_location TEXT NOT NULL DEFAULT '',
             assignee_id        INTEGER REFERENCES users(id),
             created_at         TEXT NOT NULL DEFAULT (datetime('now', 'localtime')),
             updated_at         TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
         );

         CREATE TABLE IF NOT EXISTS comments (
             id         INTEGER PRIMARY KEY,
             ticket_id  INTEGER NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
             author     TEXT NOT NULL,
             is_agent   INTEGER NOT NULL DEFAULT 0,
             internal   INTEGER NOT NULL DEFAULT 0,
             body       TEXT NOT NULL,
             created_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
         );

         CREATE INDEX IF NOT EXISTS idx_tickets_status ON tickets(status);
         CREATE INDEX IF NOT EXISTS idx_comments_ticket ON comments(ticket_id);",
    )?;
    Ok(conn)
}

// ---------- Utilisateurs (techniciens) ----------

pub fn count_users(conn: &Connection) -> Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))
}

pub fn create_user(
    conn: &Connection,
    username: &str,
    display_name: &str,
    password_hash: &str,
    is_admin: bool,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO users (username, display_name, password_hash, is_admin) VALUES (?1, ?2, ?3, ?4)",
        params![username, display_name, password_hash, is_admin],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn user_credentials(conn: &Connection, username: &str) -> Result<Option<(i64, String)>> {
    conn.query_row(
        "SELECT id, password_hash FROM users WHERE username = ?1 AND active = 1",
        [username],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
}

fn row_to_user(r: &rusqlite::Row) -> Result<User> {
    Ok(User {
        id: r.get(0)?,
        username: r.get(1)?,
        display_name: r.get(2)?,
        is_admin: r.get(3)?,
        active: r.get(4)?,
    })
}

pub fn list_users(conn: &Connection) -> Result<Vec<User>> {
    let mut stmt = conn.prepare(
        "SELECT id, username, display_name, is_admin, active FROM users ORDER BY display_name",
    )?;
    stmt.query_map([], row_to_user)?.collect()
}

pub fn set_user_active(conn: &Connection, id: i64, active: bool) -> Result<()> {
    conn.execute("UPDATE users SET active = ?1 WHERE id = ?2", params![active, id])?;
    if !active {
        conn.execute("DELETE FROM sessions WHERE user_id = ?1", [id])?;
    }
    Ok(())
}

pub fn set_user_password(conn: &Connection, id: i64, password_hash: &str) -> Result<()> {
    conn.execute(
        "UPDATE users SET password_hash = ?1 WHERE id = ?2",
        params![password_hash, id],
    )?;
    Ok(())
}

// ---------- Sessions ----------

pub fn create_session(conn: &Connection, token: &str, user_id: i64) -> Result<()> {
    conn.execute("DELETE FROM sessions WHERE expires_at < datetime('now')", [])?;
    conn.execute(
        "INSERT INTO sessions (token, user_id, expires_at) VALUES (?1, ?2, datetime('now', '+12 hours'))",
        params![token, user_id],
    )?;
    Ok(())
}

pub fn session_user(conn: &Connection, token: &str) -> Result<Option<User>> {
    conn.query_row(
        "SELECT u.id, u.username, u.display_name, u.is_admin, u.active
         FROM sessions s JOIN users u ON u.id = s.user_id
         WHERE s.token = ?1 AND s.expires_at > datetime('now') AND u.active = 1",
        [token],
        row_to_user,
    )
    .optional()
}

pub fn delete_session(conn: &Connection, token: &str) -> Result<()> {
    conn.execute("DELETE FROM sessions WHERE token = ?1", [token])?;
    Ok(())
}

// ---------- Tickets ----------

const TICKET_SELECT: &str = "SELECT t.id, t.token, t.title, t.description, t.category, t.priority, t.status,
        t.requester_name, t.requester_email, t.requester_location, t.assignee_id, u.display_name,
        t.created_at, t.updated_at
     FROM tickets t LEFT JOIN users u ON u.id = t.assignee_id";

fn row_to_ticket(r: &rusqlite::Row) -> Result<Ticket> {
    Ok(Ticket {
        id: r.get(0)?,
        token: r.get(1)?,
        title: r.get(2)?,
        description: r.get(3)?,
        category: r.get(4)?,
        priority: r.get(5)?,
        status: r.get(6)?,
        requester_name: r.get(7)?,
        requester_email: r.get(8)?,
        requester_location: r.get(9)?,
        assignee_id: r.get(10)?,
        assignee_name: r.get(11)?,
        created_at: r.get(12)?,
        updated_at: r.get(13)?,
    })
}

pub fn create_ticket(conn: &Connection, t: &NewTicket, token: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO tickets (token, title, description, category, priority,
                              requester_name, requester_email, requester_location)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            token,
            t.title,
            t.description,
            t.category,
            t.priority,
            t.requester_name,
            t.requester_email,
            t.requester_location
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn ticket_by_id(conn: &Connection, id: i64) -> Result<Option<Ticket>> {
    conn.query_row(&format!("{TICKET_SELECT} WHERE t.id = ?1"), [id], row_to_ticket)
        .optional()
}

pub fn ticket_by_token(conn: &Connection, token: &str) -> Result<Option<Ticket>> {
    conn.query_row(&format!("{TICKET_SELECT} WHERE t.token = ?1"), [token], row_to_ticket)
        .optional()
}

pub fn list_tickets(conn: &Connection, f: &TicketFilter) -> Result<Vec<Ticket>> {
    let mut sql = format!("{TICKET_SELECT} WHERE 1=1");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(s) = &f.status {
        args.push(Box::new(s.clone()));
        sql += &format!(" AND t.status = ?{}", args.len());
    } else if f.open_only {
        sql += " AND t.status NOT IN ('resolu', 'ferme')";
    }
    if let Some(p) = &f.priority {
        args.push(Box::new(p.clone()));
        sql += &format!(" AND t.priority = ?{}", args.len());
    }
    if f.unassigned {
        sql += " AND t.assignee_id IS NULL";
    } else if let Some(a) = f.assignee {
        args.push(Box::new(a));
        sql += &format!(" AND t.assignee_id = ?{}", args.len());
    }
    if let Some(q) = &f.search {
        args.push(Box::new(format!("%{q}%")));
        let n = args.len();
        sql += &format!(
            " AND (t.title LIKE ?{n} OR t.description LIKE ?{n} OR t.requester_name LIKE ?{n} \
               OR t.requester_email LIKE ?{n} OR CAST(t.id AS TEXT) = TRIM(?{n}, '%'))"
        );
    }
    sql += " ORDER BY CASE t.priority WHEN 'critique' THEN 0 WHEN 'haute' THEN 1
                      WHEN 'normale' THEN 2 ELSE 3 END, t.updated_at DESC LIMIT 500";

    let mut stmt = conn.prepare(&sql)?;
    let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
    stmt.query_map(refs.as_slice(), row_to_ticket)?.collect()
}

pub fn status_counts(conn: &Connection) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare("SELECT status, COUNT(*) FROM tickets GROUP BY status")?;
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
}

pub fn update_ticket(
    conn: &Connection,
    id: i64,
    status: &str,
    priority: &str,
    category: &str,
    assignee_id: Option<i64>,
) -> Result<()> {
    conn.execute(
        "UPDATE tickets SET status = ?1, priority = ?2, category = ?3, assignee_id = ?4,
                updated_at = datetime('now', 'localtime') WHERE id = ?5",
        params![status, priority, category, assignee_id, id],
    )?;
    Ok(())
}

pub fn set_status(conn: &Connection, id: i64, status: &str) -> Result<()> {
    conn.execute(
        "UPDATE tickets SET status = ?1, updated_at = datetime('now', 'localtime') WHERE id = ?2",
        params![status, id],
    )?;
    Ok(())
}

// ---------- Commentaires ----------

pub fn add_comment(
    conn: &Connection,
    ticket_id: i64,
    author: &str,
    is_agent: bool,
    internal: bool,
    body: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO comments (ticket_id, author, is_agent, internal, body) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![ticket_id, author, is_agent, internal, body],
    )?;
    conn.execute(
        "UPDATE tickets SET updated_at = datetime('now', 'localtime') WHERE id = ?1",
        [ticket_id],
    )?;
    Ok(())
}

pub fn comments(conn: &Connection, ticket_id: i64, include_internal: bool) -> Result<Vec<Comment>> {
    let mut stmt = conn.prepare(
        "SELECT author, is_agent, internal, body, created_at FROM comments
         WHERE ticket_id = ?1 AND (?2 OR internal = 0) ORDER BY id",
    )?;
    stmt.query_map(params![ticket_id, include_internal], |r| {
        Ok(Comment {
            author: r.get(0)?,
            is_agent: r.get(1)?,
            internal: r.get(2)?,
            body: r.get(3)?,
            created_at: r.get(4)?,
        })
    })?
    .collect()
}
