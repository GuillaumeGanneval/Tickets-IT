# Support IT – outil de ticketing

Application web en Rust (axum + SQLite) qui centralise les demandes informatiques.

## Fonctionnement

**Demandeurs (sans compte)**
- `/` : formulaire de nouvelle demande (nom, e-mail, service, catégorie, urgence, objet, description).
- Après l'envoi, ils obtiennent un **lien de suivi privé** (`/suivi/<jeton>`) pour suivre le statut,
  échanger avec le support, et confirmer/fermer le ticket une fois résolu.
- S'ils répondent sur un ticket « Résolu » ou « En attente », celui-ci repasse « En cours ».

**Techniciens (`/login`)**
- Tableau de bord avec compteurs par statut, filtres (statut, priorité, assignation) et recherche.
- Fiche ticket : réponse au demandeur, **notes internes** invisibles du demandeur,
  changement de statut / priorité / catégorie / assignation, bouton « Me l'assigner ».
- Historique automatique des modifications (en note interne).

**Administrateurs**
- `/agent/utilisateurs` : création de comptes techniciens, changement de mot de passe, désactivation.

Statuts : Nouveau → En cours → En attente → Résolu → Fermé.

## Lancer

```bash
cargo build --release
ADMIN_PASSWORD='un-mot-de-passe-solide' ./target/release/tickets-it
```

Au premier lancement, un compte `admin` est créé (si `ADMIN_PASSWORD` n'est pas défini,
un mot de passe est généré et affiché dans la console).

| Variable             | Défaut                  | Rôle                                         |
|----------------------|-------------------------|----------------------------------------------|
| `TICKETING_DB`       | `tickets.db`            | Fichier de base SQLite                       |
| `TICKETING_ADDR`     | `0.0.0.0:8080`          | Adresse d'écoute                             |
| `TICKETING_BASE_URL` | `http://localhost:8080` | URL publique, utilisée dans les liens de suivi |
| `ADMIN_PASSWORD`     | (généré)                | Mot de passe du compte admin initial         |

## Sécurité / mise en production
- Mots de passe hachés avec Argon2id, sessions par cookie `HttpOnly; SameSite=Strict` (12 h).
- Toutes les données affichées sont échappées (pas d'injection HTML).
- Placez l'application derrière un reverse proxy HTTPS (nginx, Caddy…) et sauvegardez `tickets.db`.

## Tester dans GitHub Codespaces

Bouton **Code → Codespaces → Create codespace**, attendre la fin de la préparation, puis :

```bash
cargo run
```

Ouvrir le port 8080 proposé, et se connecter sur `/login` avec `admin` / `test1234`
(mot de passe de test défini dans `.devcontainer/devcontainer.json`, uniquement pour Codespaces).
