# tessera-desossage — module « ville vide » piloté par config (redscript)

Module **redscript** du modset client Tessera qui **vide Night City** de sa vie/ambiance/quêtes
vanilla : le serveur Tessera est autoritaire et **repeuple** ensuite. Indépendant du netcode C++.

**Plateforme :** Windows-only (redscript compile en jeu au lancement ; erreurs dans
`<jeu>/r6/logs/redscript_rCURRENT.log`). Se conçoit/écrit sur macOS, se **teste en jeu**.

## Le seul endroit à toucher : la config

Tout est piloté par `DesossageConfig.Default()` dans
[`DesossageConfig.reds`](overlay/r6/scripts/Tessera/desossage/DesossageConfig.reds). **Un champ par
système**, chacun une `DesossageEntry { active, density }`. Défaut = **tout coupé** (monde vide).

- **Rallumer un système** : passer `active: true` (+ une `density`), ex. réintroduire des
  cyberpsychoses clairsemées → `c.cyberpsychos = DesossageEntry.New(true, 0.3);`.
- **Régler le cycle jour/nuit** : `c.dayNightCycleScale` (1.0 normal · 2.0 = 2× plus long · 0.0 figé).
- **Ajouter un futur type de rencontre** : un champ dans `DesossageConfig` + un levier dans
  l'applicateur — sans toucher au reste.

## Structure

| Fichier | Rôle |
|---|---|
| `DesossageConfig.reds` | la config centrale + défauts (monde vide) |
| `DesossageSystem.reds` | applicateur (`ScriptableSystem`) + déclencheur au chargement (`OnGameAttached`) |
| `DesossagePopulation.reds` | leviers piétons / trafic / transit |
| `DesossageOrder.reds` | levier police (`PreventionSystem`) + sécurité ambiante |
| `DesossageDevices.reds` | voyage rapide (kiosques) / vendeurs / distributeurs / interactables |
| `DesossageEvents.reds` | rencontres (par type) / quêtes / tutoriels |
| `DesossageWorld.reds` | échelle du cycle jour/nuit |
| `DesossageConsole.reds` | bascule un levier en jeu via la console CET, sans rebuild (voir « Tester ») |

## État des leviers (mis à jour 2026-07-06)

Table de suivi — **la** référence pour éviter de re-creuser un levier déjà tranché. Champs :
mécanisme réel cité, statut, testé en jeu ou non, limites connues. Détails/sources complètes dans
les commentaires du fichier `.reds` concerné (lien dans « Structure » ci-dessus).

| Levier | Statut | Mécanisme | Testé en jeu | Limites / notes |
|---|---|---|---|---|
| `police` | ✅ Réel | `@wrapMethod(PreventionSystem) OnAttach()` | ✅ 2026-07-05 (plus d'étoiles) | BOOT ONLY (recharge nécessaire pour changer) |
| `ambientSecurity` | ⚠️ Partiel | `@wrapMethod(SecurityTurretControllerPS) GetActions` | ✅ 2026-07-05 (menu quickhack absent) | Coupe l'interaction, pas la détection IA — tourelle peut rester hostile |
| `gangHostility` | ✅ Réel | attitude/relations gangs | ✅ 2026-07-05 | Décoché = gangs non hostiles (relations neutres) |
| `pedestrians` / `traffic` | ✅ Réel — **double mécanisme** | **Primaire** : `engine/config/platform/pc/user.ini` `[Crowd]` (`Enabled`/`EnablePedestrians`/`EnableVehicles` = false) — lu au chargement, s'applique à toute la carte, jamais réinitialisé par le streaming. **Backup défensif** : `CommunitySystem.ChangeDensityModifier` + `@wrapMethod(PreventionSystem) OnDistrictAreaEntered` (réapplique à chaque changement de quartier) | ✅ ini confirmé par Lucas en jeu (2026-07-06, v2.31, plus aucun piéton/véhicule) · le hook district (backup) pas re-testé isolément | `traffic` = doublon confirmé de `pedestrians` côté redscript. L'ini résout le "ça revient en changeant de quartier" à la racine — le hook redscript reste en filet de sécurité, pas la solution principale désormais |
| `vendors` | ⚠️ Partiel | masque l'icône de rôle PNJ (`GameplayRoleComponent`) | ✅ 2026-07-05 | Masque l'icône, PAS l'interaction. Vendeurs ambiants probablement couverts par `pedestrians` (à confirmer) ; vendeurs/ripperdoc NOMMÉS = système de dialogue NPC, pas de toggle trouvé (`VendorComponent` = getters seulement) |
| `transit` (métro/NCART) | 🔴 Stub, aucun effet | — | — | **Volontairement laissé sans effet** : le métro doit rester payant/normal (demande 2026-07-06), ne PAS lui donner de comportement — confirmé indépendant de `fastTravel` |
| `fastTravel` | ✅ Réel | `FastTravelSystem.ManageFastTravelLock` | — | BOOT ONLY. Ne touche pas le métro (système distinct, confirmé) |
| `vendingDevices` | ✅ Réel | `@wrapMethod(VendingMachineControllerPS) GetActions` | ✅ | Couvre boissons/nourriture. Pas encore fait : distributeurs d'armes, droppoints (classes PS sœurs) |
| `worldInteractables` | ✅ Réel | `@wrapMethod(ScriptableDeviceComponentPS) GetActions` (hook générique, base commune) | ⚠️ jamais testé | Couvre points d'accès/hackables **et**, découvert 2026-07-06, l'écran de statut de loyer d'appartement (`ApartmentScreenControllerPS` hérite de la même base) — gratuit, pas de code dédié nécessaire. Ne couvre PAS l'achat d'un nouvel appartement (classe distincte non trouvée) |
| `questTriggers` | ⚠️ Partiel | `questPhoneManager.ApplyPhoneCallRestriction` | ✅ (icône radio déverrouillée si décoché) | Bloque les appels fixers, pas les déclencheurs de zone/PNJ ni les quêtes en général |
| `tutorials` | 🔴 Stub confirmé mort | `questTutorialManager` existe mais ne ferme qu'un overlay déjà ouvert | — | Aucune piste RTTI restante |
| `ncpdHustles` | 🔴 Stub confirmé mort | — | — | Aucune classe « Hustle »/« CrimeSpawn » dans le jeu (RTTI épuisé) |
| `randomEncounters` | 🔴 Stub confirmé mort | — | — | Absent du RTTI |
| `cyberpsychos` | 🔴 Stub confirmé mort | — | — | Absent du RTTI — nécessiterait une édition de données TweakDB, pas un hook |
| `mapMarkers` | ⚠️ Partiel | nettoyage ponctuel carte/minimap + masque icône vigilance PNJ | ✅ 2026-07-05 | Nettoyage ponctuel seulement (pas garanti map-wide/persistant) — même famille de risque que le fix district : candidat prioritaire pour la migration ini (2026-07-06) |
| `dayNightCycleScale` | 🔴 Stub, pas assez sûr | `gameTimeSystem.SetTimeDilation` existe mais affecte aussi le joueur (mauvaise sémantique) | — | Pas codé à l'aveugle, en attente d'une meilleure piste |

**Nouveau (2026-07-06) — pas encore trouvé** : achat d'un nouvel appartement (real-estate, distinct
de l'écran de statut de loyer déjà couvert), interaction vendeur/ripperdoc (système de dialogue,
pas le pattern « device PS »).

**Direction actée (2026-07-06)** : pour tout levier où le mécanisme réel est un système
communautaire/streaming (piétons/trafic, marqueurs carte...), le risque « reset au changement de
quartier/zone » est le même que celui déjà rencontré et corrigé en réactif pour `pedestrians`. Un
hook réactif par cas n'est pas jugé assez robuste (« trop le bordel ») — direction : migrer vers
config moteur ini (`engine/config/platform/pc/*.ini`) ou l'API CET `GameOptions`, qui s'appliquent
une fois pour toute la carte au chargement plutôt que d'être réappliquées à chaque déclencheur.
**Piétons/trafic fait** (2026-07-06) : `[Crowd]` dans `user.ini`, fichier réel fourni par Lucas,
confirmé en jeu. Process établi pour la suite : Nexus bloque le fetch direct (403), Lucas
télécharge le mod et colle le contenu réel de l'ini dans `tools/mod-research/inbox/` (jamais deviner
des clés, cf. `feedback-nexus-mods-ask-lucas`) — encore à récupérer : marqueurs carte/mini-carte
(« no map icons »), vendeurs/ripperdoc, police (pistes mods : « Disabled Crowd » #175,
« Realistic Traffic Density » #6457, « CP77 Ini Tweaker » #15973 pour `GameOptions` à la volée).

**Leçon (persistante) :** un symbole plausible mais non vérifié (`CanPreventionReactToInput`,
jamais confirmé) a cassé le jeu entier (crash au lancement, plusieurs cycles de réinstallation
Windows). Toute future implémentation doit citer une source réelle (mod publié, script décompilé,
dump RTTI local — `tools/nativedb/search.py`) — sinon rester en stub et le documenter comme tel
dans cette table.

## Où ça se déploie

Overlay enraciné à la racine du jeu : `<racine Cyberpunk>/r6/scripts/Tessera/desossage/*.reds`.
Empaqueté dans le modset client par `tessera-release`, installé par le launcher (overlay générique,
confirmé). redscript (dépendance toolchain) compile les `.reds` au lancement.

## Tester

1. Emballer un modset dev incluant ce module, le publier sur le canal dev, l'installer via le launcher.
2. Lancer le jeu, charger dans le monde. Lire `r6/logs/redscript_rCURRENT.log` :
   - **compile clean** (aucune erreur `Tessera.Desossage`) ;
   - lignes `[Tessera/Desossage] système attaché`, `application des leviers…`, puis les stubs.
3. (Après pinning des leviers) observer : rues vides, pas de police/vendeurs/kiosques/quêtes ; et
   **smoke-test du bouton** : `pedestrians = DesossageEntry.New(true, 0.3)` → piétons clairsemés.

## Itérer sans rebuild : console CET

`DesossageConsole.reds` expose `Tessera_SetLever` sur le joueur, appelable depuis la console CET
(`~` par défaut) une fois en jeu, session chargée :

```lua
Game.GetPlayer():Tessera_SetLever("police", true, 0.0)
Game.GetPlayer():Tessera_SetLever("pedestrians", true, 0.3)
Game.GetPlayer():Tessera_SetLever("dayNightCycleScale", true, 2.0)
```

Ne persiste pas entre rechargements (repart de `DesossageConfig.Default()`) — pour un réglage
permanent, toujours éditer `DesossageConfig.reds` + rebuild.
