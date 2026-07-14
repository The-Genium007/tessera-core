# ADR 0009: Heure du monde (jour/nuit) — préparer le client-mod pour l'autorité serveur

- **Statut :** proposé — partie client préparée et testée en solo (2026-07-05), synchronisation
  réseau **différée** (décision utilisateur : « on prépare les mods pour l'avenir », le système
  serveur viendra plus tard)
- **Date :** 2026-07-05

## Contexte

Pendant la session de test désossage sur la machine Windows (voir `MISSION.md`,
`tools/nativedb/findings.md`), le slider `dayNightCycleScale` du panneau `TesseraDesossage`
s'est confirmé être un stub sans aucun effet — `gameTimeSystem.SetTimeDilation(...)` est la seule
piste native trouvée pour une échelle de vitesse, mais c'est un ralenti global qui affecte aussi
le mouvement/combat du joueur (contraire à la contrainte de design : seuls temps/météo doivent
varier).

Une piste plus solide a été trouvée et testée en jeu avec succès : `gameTimeSystem
.SetGameTimeByHMS(hours, minutes, seconds, reason)` — saut direct à une heure précise, sans
toucher au joueur ni au combat. Confirmé par le script décompilé officiel
(`CDPR-Modding-Documentation/Cyberpunk-Scripts/scripts/core/systems/timeSystem.script`) et
plusieurs mods publiés réels (`CyanideX/NovaCityTools`, `MaximiliumM/appearancemenumod`,
`Avi6481/EasyTrainers`).

Suite à ce test, l'utilisateur a demandé : pouvoir synchroniser l'heure du monde entre tous les
clients connectés via le serveur autoritaire (cohérent avec le principe déjà posé dans
`CLAUDE.md` : *« Le serveur Rust est autoritaire »*). Décision explicite : préparer le mod
client MAINTENANT pour cet usage futur, documenter le design ici, et construire le système
serveur (protocole + logique Rust) **plus tard**, dans une session dédiée.

## Décision

**Découpler "comment on apprend l'heure" de "comment on l'applique en jeu"**, pour que le futur
message réseau n'ait qu'à brancher sur une fonction déjà existante et déjà testée, sans retoucher
la partie redscript qui parle au moteur.

### Ce qui existe déjà côté client-mod (implémenté et validé en jeu 2026-07-05)

- `Tessera_DoJumpToTime(game: GameInstance, hour: Int32, minute: Int32) -> Void`
  (`DesossageWorld.reds`) — LE point d'entrée unique qui appelle
  `GameInstance.GetTimeSystem(game).SetGameTimeByHMS(hour, minute, 0, n"tessera_desossage")`.
  Granularité déjà à la minute (le paramètre `seconds` de l'API native est disponible mais pas
  encore exposé par ce wrapper — trivial à ajouter si besoin, cf. section Conséquences).
- `Tessera_JumpToTime(hour: Int32, minute: Int32)` (`@addMethod(PlayerPuppet)`,
  `DesossageConsole.reds`) — expose la fonction ci-dessus à la console CET / au panneau Lua, sans
  dépendre de `eval` (confirmé cassé côté sandbox Lua CET, cf. `tools/game-harness/README.md`).
- Boutons "Midi (12h00)" / "Minuit (00h00)" dans `TesseraDesossage/init.lua` — déclenchement
  manuel pour test solo, pas destinés à survivre tels quels une fois la synchro serveur en place
  (remplacés à terme par un affichage lecture-seule de l'heure reçue du serveur, cf. ci-dessous).

### Ce qui reste à faire (différé, hors scope de cette session)

1. **Message protocole** (`tessera-core/protocol/schema/protocol.fbs`) : ajouter une table
   `WorldTime { hour:ubyte; minute:ubyte; second:ubyte; }` (ou un timestamp epoch unique côté
   serveur, à trancher au moment de l'implémentation — voir Alternatives) au `union ServerMsg`
   existant (`Snapshot`, `Kicked`). Flux **unidirectionnel serveur → client** uniquement — l'heure
   du monde n'est pas un input joueur, donc pas de table `ClientMsg` correspondante.
2. **Autorité côté serveur** (`tessera-core/server/src/world.rs` ou un nouveau module dédié) :
   le serveur possède l'horloge de référence (probablement dérivée du tick de simulation plutôt que
   de l'horloge système, pour rester déterministe/rejouable) et diffuse `WorldTime` :
   - à la connexion d'un client (état initial), et
   - périodiquement ou sur changement significatif (à définir — un envoi toutes les N secondes de
     jeu simulé suffit, pas besoin de le caler sur chaque tick réseau).
3. **Réception côté client** (nouveau, dans `tessera-core/client-mod/` ou le futur module réseau
   du client-mod, PAS dans `tessera-desossage/` qui reste un outil de test/désossage) : sur
   réception d'un message `WorldTime`, appeler `Tessera_DoJumpToTime` (ou un équivalent non lié au
   désossage — voir Conséquences) avec les valeurs reçues.
4. **Décider de l'application progressive vs instantanée** : `SetGameTimeByHMS` fait un saut sec.
   Si le décalage client/serveur est petit (quelques minutes de jeu), un saut est probablement
   imperceptible ; si un client rejoint en cours de partie avec un delta de plusieurs heures
   in-game, un saut instantané peut surprendre visuellement (jour → nuit d'un coup). Non résolu ici
   — à trancher avec un test en jeu au moment de l'implémentation (cf. Alternatives).

## Conséquences

- **Positif :** la partie la plus incertaine (trouver le bon symbole RTTI/redscript, confirmer
  qu'il ne casse pas le compile, confirmer qu'il n'affecte pas le joueur) est déjà faite et
  validée en jeu — le futur travail réseau n'a qu'à brancher dessus.
- **À anticiper :** `Tessera_DoJumpToTime` vit aujourd'hui dans `tessera-desossage/` (dossier de
  test/désossage, pas destiné à un modset publié tel quel — cf. `CLAUDE.md`/`README.md` du
  client-mod). Le jour où la synchro serveur est construite, il faudra probablement **déplacer ou
  dupliquer** ce point d'entrée vers un module client-mod pérenne (hors désossage), pour ne pas
  faire dépendre une fonctionnalité gameplay permanente d'un outil de test.
- **À anticiper :** `SetGameTimeByHMS` ne prend pas de `second` depuis notre wrapper actuel
  (toujours `0`) — étendre la signature à 3 paramètres (h/m/s) le jour où une précision à la
  seconde est utile pour la synchro (probablement pas nécessaire, l'API réseau enverra sans doute
  une cadence de sync plus grossière que la seconde).
- **Risque non résolu :** aucune piste native trouvée pour *figer* l'heure sans toucher au joueur
  (l'équivalent "pause" de `gameTimeSystem` n'a pas été creusé — `SetPausedState` existe dans le
  dump RTTI mais son périmètre exact, gameplay ou horloge seule, n'a pas été vérifié). Pertinent
  si le serveur veut un mode "figé" plutôt que "défilement synchronisé".

## Alternatives considérées

- **Timestamp epoch unique plutôt que h/m/s séparés dans le message réseau** : plus compact, mais
  demande une conversion côté client (epoch → h/m/s) avant d'appeler `SetGameTimeByHMS`. Pas
  tranché — dépend de si le serveur veut aussi encoder une date calendaire complète (jour du cycle
  jour/nuit) ou juste l'heure du jour. À décider avec le design du système serveur, pas maintenant.
- **Interpolation cliente continue (le client fait défiler l'heure localement entre deux syncs,
  plutôt que d'attendre un message pour chaque minute)** : plus fluide visuellement, mais plus
  complexe (il faut un rythme d'écoulement local qui ne dérive pas trop de l'autorité serveur).
  Écarté pour l'instant — un saut périodique simple (option retenue implicitement ci-dessus) est
  suffisant tant que la fréquence de sync reste raisonnable ; à revisiter si les sauts sont trop
  visibles en pratique.
- **Garder `dayNightCycleScale`/`SetTimeDilation` comme mécanisme de synchro** : écarté dès le
  départ (avant même cette ADR) à cause de l'effet de bord sur le joueur/combat — confirmé de
  nouveau ici comme non-retenu.
