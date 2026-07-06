# tessera-playtest-save — lancement direct sur une sauvegarde dédiée

Fait partie de la chaîne "clic Play → dans le monde, sans rien voir" (spec
`docs/superpowers/specs/2026-07-05-playtest-shards-design.md` §#3) : avec `-skipStartScreen` (déjà
géré côté launch args), ce module couvre le morceau restant — sauter le menu principal et charger
directement une sauvegarde connue, sans passer par Continuer/Nouvelle partie.

**Plateforme :** Windows-only (CET Lua, s'exécute au chargement du jeu ; installation par le
launcher = Rust, testable sur toute plateforme mais n'a d'effet réel que sur Windows). Conçu/écrit
sur macOS, testé en jeu.

## Structure

| Élément | Rôle |
|---|---|
| `overlay/bin/x64/plugins/cyber_engine_tweaks/mods/TesseraAutoLoad/init.lua` | hook `SingleplayerMenuGameController.OnSavesForLoadReady` → charge la save ciblée par nom |
| `overlay/_tessera_playtest_save/TesseraPlaytest/` | **la sauvegarde elle-même** — chemin de transit dans le zip d'overlay, jamais lu par le jeu ; le launcher la déplace vers `Saved Games` après sync (voir ci-dessous), et nettoie ce dossier de transit de l'install du jeu |

## État (2026-07-06)

- **Hook d'auto-chargement** : codé, sourcé (2 mods réels indépendants : `Nats-ji/CP77-Skip-Main-Menu`
  CET Lua, `psiberx/cp2077-playground` redscript ; méthode `LoadSaveInGame` confirmée par dump
  RTTI local). Cible le dossier `TesseraPlaytest` (nom volontairement distinctif, pas
  `ManualSave-N`, pour ne jamais entrer en collision avec les sauvegardes propres du joueur).
  **Jamais testé en jeu.**
- **Fichier de sauvegarde** : save PERSO de Lucas (pas un mod Nexus tiers — la première tentative,
  "Skip to Act 2 Save and more" #27436, avait une licence "pas de redistribution", écartée).
  `buildPatch: 2.31` (match exact avec notre cible), StreetKid, fin Acte 2, non moddée,
  jouée le 2026-07-06. Committée dans `overlay/_tessera_playtest_save/TesseraPlaytest/` — aucun
  souci de licence, c'est son propre contenu.
- **Package modset** : ajouté à `distribution/modset.packages.toml` (`tessera-playtest-save`,
  `required = true`) — sera inclus dans le prochain build de modset dev/stable.
- **Installation côté launcher** : câblée. `sync_modset` (Rust, `src-tauri/src/lib.rs`) déplace
  `_tessera_playtest_save/TesseraPlaytest/` de l'install du jeu vers
  `%USERPROFILE%\Saved Games\CD Projekt Red\Cyberpunk 2077\TesseraPlaytest\` juste après que la
  transaction d'overlay a réussi, puis supprime le dossier de transit (il n'a rien à faire dans
  l'install du jeu). Échec non bloquant : si l'installation de la save rate, le sync du reste du
  modset reste valide (juste un menu normal au lancement plutôt que l'auto-load).
  `cargo test` vert (27/27), jamais testé en conditions réelles (build Windows).

## À faire ensuite

- Build Windows + test en jeu réel : la save doit apparaître dans la liste des sauvegardes sous le
  nom `TesseraPlaytest`, et l'auto-load doit se déclencher au démarrage.
- Si le nom exact affiché dans `OnSavesForLoadReady` diffère du nom de dossier, ajuster
  `TARGET_SAVE_NAME` dans `init.lua`.
