# tessera-skip-intro — saute la vidéo d'intro (logos CDPR)

Fait partie de la même chaîne "clic Play → dans le monde, sans rien voir" que
`tessera-playtest-save` (skip menu principal + auto-load) : ce module couvre le morceau qui
restait — la vidéo/logos au tout lancement du jeu, avant même le menu principal.

**Plateforme :** Windows-only (redscript compile en jeu au lancement ; erreurs dans
`<jeu>/r6/logs/redscript_rCURRENT.log`). Conçu sur macOS, à tester en jeu.

## Structure

| Fichier | Rôle |
|---|---|
| `overlay/r6/scripts/NoIntroVideos.reds` | hook `OnInitialize` sur le contrôleur d'écran de splash (`SplashScreenLoadingScreenLogicController` / `inkSplashScreenLoadingScreenLogicController`) — renomme les 4 animations d'intro (logos, message localisé, intro jeu, version longue) vers `after_skip_pressed`, donc elles se jouent déjà "comme si Skip avait été pressé" |

Deux variantes dans le même fichier (`@if(ModuleExists("Codeware"))`) : avec Codeware, `@addMethod`
sur la classe de base et vérif `IsA`. Sans Coderound, classe déclarée `native` directement (les
champs `logosTrainAnimation`/etc. existent déjà nativement dans le jeu, ce ne sont pas des ajouts).

## Provenance

Fichier fourni par Lucas (déposé dans `tools/mod-research/inbox/no video intro/`, 2026-07-07) —
aucun fichier de licence/readme n'accompagnait le `.reds` dans l'Inbox. Technique générique et
courte (renommage de 4 champs d'animation déjà exposés par le jeu), commune à plusieurs mods de
saut d'intro publiés sur Nexus — pas un asset CDPR, pas de contenu extrait du jeu. Si une
restriction de licence apparaît plus tard pour ce fichier précis, revoir avant toute redistribution
plus large (cf. `feedback-nexus-mods-ask-lucas` : ne jamais deviner, toujours vérifier auprès de
Lucas/de la page source en cas de doute).

## État (2026-07-07)

- Intégré tel que fourni, pas de modification du contenu.
- **Jamais testé en jeu.** À vérifier au prochain lancement : `redscript_rCURRENT.log` doit rester
  clean (aucune erreur `NoIntroVideos`/`SplashScreenLoadingScreenLogicController`), et la vidéo
  d'intro/les logos CDPR doivent être sautés dès le tout premier lancement (avant le menu principal).
- Ajouté à `distribution/modset.packages.toml` (`tessera-skip-intro`, `required = true`).

## Où ça se déploie

Overlay enraciné à la racine du jeu : `<racine Cyberpunk>/r6/scripts/NoIntroVideos.reds`. Empaqueté
dans le modset client par `tessera-release`, installé par le launcher (overlay générique).
redscript compile le fichier au lancement.
