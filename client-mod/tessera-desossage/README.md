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
| `DesossagePopulation.reds` | leviers piétons (couvre aussi trafic véhicules, doublon retiré 2026-07-07) / transit |
| `DesossageOrder.reds` | levier police (`PreventionSystem`) + sécurité ambiante |
| `DesossageDevices.reds` | voyage rapide (kiosques) / vendeurs / distributeurs / interactables |
| `DesossageEvents.reds` | rencontres (par type) / quêtes / tutoriels |
| `DesossageWorld.reds` | échelle du cycle jour/nuit |
| `DesossageConsole.reds` | bascule un levier en jeu via la console CET, sans rebuild (voir « Tester ») |

## ⚠️ Avant de toucher un `@wrapMethod` : le dump RTTI ne donne PAS les noms redscript

Deux vagues d'échecs de compilation (2026-07-05, puis **récidive le 2026-07-15**) ont la même
cause : un nom de classe pris dans le dump RTTI (`tools/nativedb`, qui liste les noms **natifs**)
et écrit tel quel en redscript, qui ne connaît que l'**alias importé court**.

| Écrit (RTTI) | Correct (redscript) | Résultat de l'erreur |
|---|---|---|
| `gamemappinsMappinSystem` | `MappinSystem` | 3 échecs, 2026-07-05 |
| `gameuiStealthMappinController` | `StealthMappinController` | `[UNRESOLVED_REF]`, 2026-07-15 |

Et la règle **n'est pas uniforme** : `gameTimeSystem` garde son préfixe. Il n'y a donc pas de
transformation mécanique à appliquer — chaque nom se confirme au cas par cas.

**Source de vérité pour un nom de classe** : le script décompilé officiel
(`CDPR-Modding-Documentation/Cyberpunk-Scripts` — chercher `import class X` / `class X extends`),
ou un mod publié qui compile. Le dump RTTI sert à *trouver* un candidat et à vérifier les
**méthodes** ; il ne dit rien des **noms** visibles côté redscript, ni des méthodes `private`
scriptées (absentes du RTTI — c'est ce qui a fait croire à tort que `StealthMappinController`
n'existait pas). Détail complet : `tools/nativedb/findings.md`.

Corollaire vécu : un script de vérification qui n'interroge que le dump **valide à tort** et
casse du code correct. Ne pas "corriger" un nom court vers un nom préfixé sans preuve.

## État des leviers (mis à jour 2026-07-07)

Table de suivi — **la** référence pour éviter de re-creuser un levier déjà tranché. Champs :
mécanisme réel cité, statut, testé en jeu ou non, limites connues. Détails/sources complètes dans
les commentaires du fichier `.reds` concerné (lien dans « Structure » ci-dessus).

| Levier | Statut | Mécanisme | Testé en jeu | Limites / notes |
|---|---|---|---|---|
| `police` | ✅ Réel | `@wrapMethod(PreventionSystem) OnAttach()` | ✅ 2026-07-05 (plus d'étoiles) | BOOT ONLY (recharge nécessaire pour changer) |
| `ambientSecurity` | ⚠️ Partiel | `@wrapMethod(SecurityTurretControllerPS) GetActions` | ✅ 2026-07-05 (menu quickhack absent) | Coupe l'interaction, pas la détection IA — tourelle peut rester hostile |
| `gangHostility` | ✅ Réel | attitude/relations gangs | ✅ 2026-07-05 | Décoché = gangs non hostiles (relations neutres) |
| `pedestrians` | ✅ Réel | `engine/config/platform/pc/user.ini` `[Crowd]` (`Enabled`/`EnablePedestrians`/`EnableVehicles` = false) — lu au chargement, s'applique à toute la carte, jamais réinitialisé par le streaming | ✅ ini confirmé par Lucas en jeu (2026-07-06, v2.31, plus aucun piéton/véhicule) | Couvre aussi le trafic véhicules (même nœud de spawn, confirmé en jeu 2026-07-05). Le levier `traffic` dédié (config+UI+console) a été **retiré 2026-07-07** — doublon confirmé, plus de code à maintenir |
| `vendors` | ⚠️ Partiel | icône masquée (`GameplayRoleComponent`) **+** commerce bloqué (`@wrapMethod(MenuScenario_Vendor) OnEnterScenario` → `GotoIdleState()`, ajouté 2026-07-07) | icône ✅ 2026-07-05 ; blocage commerce ⚠️ jamais testé | Le PNJ reste visible et interactible en dialogue générique (pas de piste pour le despawn), mais son commerce (vendor hub/ripperdoc/craft) ne s'ouvre plus. Vendeurs ambiants probablement couverts par `pedestrians` (hypothèse, à confirmer) |
| `transit` (métro/NCART) | 🔴 Stub, aucun effet | — | — | **Volontairement laissé sans effet** : le métro doit rester payant/normal (demande 2026-07-06), ne PAS lui donner de comportement — confirmé indépendant de `fastTravel` |
| `fastTravel` | ✅ Réel | `FastTravelSystem.ManageFastTravelLock` | — | BOOT ONLY. Ne touche pas le métro (système distinct, confirmé) |
| `vendingDevices` | ✅ Réel | `@wrapMethod(VendingMachineControllerPS) GetActions` + `@wrapMethod(DropPointControllerPS) GetActions` (2 hooks) | boissons/nourriture ✅ ; droppoints ⚠️ jamais testés | `WeaponVendingMachineControllerPS` **hérite** de `VendingMachineControllerPS` et n'override PAS `GetActions` (vérifié au dump RTTI 2026-07-15) : son hook dédié échouait en `[UNRESOLVED_METHOD]` et a été retiré — les distributeurs d'armes sont couverts par le wrap du parent (dispatch virtuel) |
| `worldInteractables` | ✅ Réel | `@wrapMethod(ScriptableDeviceComponentPS) GetActions` (hook générique, base commune) | ⚠️ jamais testé | Couvre points d'accès/hackables **et**, découvert 2026-07-06, l'écran de statut de loyer d'appartement (`ApartmentScreenControllerPS` hérite de la même base) — gratuit, pas de code dédié nécessaire. Ne couvre PAS l'achat d'un nouvel appartement (classe distincte non trouvée) |
| `questTriggers` | ⚠️ Partiel | `questPhoneManager.ApplyPhoneCallRestriction` | ✅ (icône radio déverrouillée si décoché) | Bloque les appels fixers, pas les déclencheurs de proximité (gigs) ni les hustles NCPD — nécessiterait un hook C++ natif sur `questSpawnManagerNodeDefinition`/`ExecuteNode` (recherche 2026-07-07, pas encore décidé) |
| `tutorials` | ✅ Réel (2026-07-07) | fact save `disable_tutorials` posé via `QuestsSystem.SetFact` (retrouvé dans un export CyberCAT réel, non préfixé par un code de quête) | ⚠️ jamais testé | Remplace l'ancienne piste `questTutorialManager` (confirmée insuffisante — ne fermait qu'un overlay déjà ouvert) |
| `airTraffic` (nouveau 2026-07-07) | ✅ Réel | fact save `air_traffic_off` via `SetFact`, même mécanisme que `tutorials` | ⚠️ jamais testé | — |
| `ncpdHustles` | 🔴 Stub confirmé mort | `questSpawnManagerNodeType`/enum `populationSpawnerObjectCtrlAction` — exécution purement native (nœud de graphe de quête) | — | Nécessiterait un hook C++ natif (`QuestPhaseInstance::ExecuteNode`) |
| `randomEncounters` | 🔴 Stub confirmé mort | idem `ncpdHustles` | — | idem |
| `cyberpsychos` | 🔴 Stub confirmé mort | idem `ncpdHustles` | — | idem |
| **PNJ statiques (groupes qui discutent, ne marchent pas)** | 🔴 Nouveau cas sans levier (signalé 2026-07-07) | Système **Community** (`worldCommunityRegistryNode`, champ `representsCrowd:Bool`) — DISTINCT du système **Crowd** que pilote `ChangeDensityModifier`/`pedestrians`. Contrôle natif seulement, via le même `questSpawnManagerNodeType` que ci-dessus | — | Recherche déléguée (Fable 5, 2026-07-07) confirmée par script décompilé (`communitySystem.script` : 4 méthodes, toutes côté Crowd) + mods publiés (Nova Crowds, No Crowds and Cars, Disabled Crowd — tous confirment ne pas retirer les PNJ fixes). Même hook C++ candidat que `ncpdHustles`/`randomEncounters`/`cyberpsychos` |
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

## Note (2026-07-16) — Gate d'observation profonde

Un instrument d'observation log-only (canaux nœuds de quête, facts, état désossage effectif,
marqueurs d'action joueur) est ajouté à ce module pour préparer un futur Palier 2 (blocage
sélectif). Voir `gate-observation-protocol.md` (ce dossier) pour le protocole de session, et
`docs/superpowers/specs/2026-07-16-desossage-campagne-gelee-observation-design.md` pour le design
complet. Ces canaux ne modifient AUCUN comportement de jeu — ils journalisent seulement.

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
