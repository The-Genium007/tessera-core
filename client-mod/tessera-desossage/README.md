# tessera-desossage — module « ville vide » piloté par config (redscript)

Module **redscript** du modset client Tessera qui **vide Night City** de sa vie/ambiance/quêtes
vanilla : le serveur Tessera est autoritaire et **repeuple** ensuite. Indépendant du netcode C++.

**Plateforme :** Windows-only (redscript compile en jeu au lancement ; erreurs dans
`<jeu>/r6/logs/redscript_rCURRENT.log`). Se conçoit/écrit sur macOS, se **teste en jeu**.

## Le seul endroit à toucher : la config

Tout est piloté par `DesossageConfig.Default()` dans
[`DesossageConfig.reds`](r6/scripts/Tessera/desossage/DesossageConfig.reds). **Un champ par
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

> **État actuel : squelette.** L'ossature (config + applicateur + déclenchement) est complète et
> **compile telle quelle** ; les corps de leviers sont des **stubs qui journalisent** leur intention.
> Les appels réels aux systèmes du jeu (marqués `PIN IN-GAME`) se pincent **en jeu**, lot par lot.

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
