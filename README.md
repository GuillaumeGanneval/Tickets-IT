# Support IT – outil de ticketing

Application web en Rust (axum + SQLite) qui centralise les demandes informatiques.

## Fonctionnement

**Demandeurs (compte personnel)**
- `/inscription` : création de compte **réservée aux adresses du domaine autorisé** (`@sias.fr` par défaut).
- Connexion sur `/login` avec l'adresse e-mail, puis :
  - `/` : formulaire de nouvelle demande (nom et e-mail repris du compte) ;
  - `/mes-demandes` : liste de toutes ses demandes avec leur statut et le technicien en charge.
- Sur chaque demande : suivi des échanges, réponse au support, confirmation/fermeture une fois résolue.
- S'ils répondent sur un ticket « Résolu » ou « En attente », celui-ci repasse « En cours ».
- Les tickets envoyés auparavant avec la même adresse sont rattachés au compte à l'inscription.

**Techniciens (`/login`, avec leur identifiant)**
- Tableau de bord avec compteurs par statut, filtres (statut, priorité, assignation) et recherche.
- Fiche ticket : réponse au demandeur, **notes internes** invisibles du demandeur,
  changement de statut / priorité / catégorie / assignation, bouton « Me l'assigner ».
- Historique automatique des modifications (en note interne).

**Administrateurs**
- `/agent/utilisateurs` (« Comptes ») : création de comptes techniciens, changement de mot de passe,
  désactivation de n'importe quel compte (technicien ou demandeur).

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
| `TICKETING_ALLOWED_DOMAIN` | `sias.fr`         | Seul domaine e-mail autorisé à créer un compte |
| `ADMIN_PASSWORD`     | (généré)                | Mot de passe du compte admin initial         |

## Sécurité / mise en production
- Mots de passe hachés avec Argon2id, sessions par cookie `HttpOnly; SameSite=Strict` (12 h).
- L'adresse e-mail n'est pas vérifiée à l'inscription (pas d'envoi d'e-mail) : le contrôle porte
  uniquement sur le domaine saisi.
- Toutes les données affichées sont échappées (pas d'injection HTML).
- Placez l'application derrière un reverse proxy HTTPS (nginx, Caddy…) et sauvegardez `tickets.db`.

## Tester dans GitHub Codespaces

Bouton **Code → Codespaces → Create codespace**, attendre la fin de la préparation, puis :

```bash
cargo run
```

Ouvrir le port 8080 proposé, et se connecter sur `/login` avec `admin` / `test1234`
(mot de passe de test défini dans `.devcontainer/devcontainer.json`, uniquement pour Codespaces).
