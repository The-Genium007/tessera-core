# C1 — reconstruction du désossage sur la doctrine d'interception

**Inventaire dressé le 2026-07-27.** Première étape demandée par le backlog (`BACKLOG-INGAME` C1) :
*« inventorier les familles réellement coupées, et pour chacune trouver son entonnoir unique »*.

La doctrine (ADR 0011–0014, actée le 2026-07-20) : **intercepter au point d'entrée, garder le
moteur, réinjecter l'autorité serveur** — plutôt que couper en bloc sans savoir ce qui est
réactivable. Le désossage actuel est antérieur à cette doctrine.

## Ce que le module coupe réellement — 10 hooks

| # | Point d'accroche | Effet | Verdict |
| --- | --- | --- | --- |
| 1 | `ScriptableDeviceComponentPS.GetActions` | `return false` | ❌ **destruction, la pire** |
| 2 | `VendingMachineControllerPS.GetActions` | `return false` | ❌ destruction |
| 3 | `DropPointControllerPS.GetActions` | `return false` | ❌ destruction |
| 4 | `SecurityTurretControllerPS.GetActions` | `return false` | ❌ destruction |
| 5 | `PreventionSystem.OnAttach` | retour anticipé | ❌ **empêche le système d'exister** |
| 6 | `AIActionHelper.TryChangingAttitudeToHostile` | `false` **si la cible est le joueur** | 🟡 ciblé, acceptable |
| 7 | `MenuScenario_Vendor.OnEnterScenario` | `GotoIdleState()` | ✅ interception propre |
| 8 | `GameplayRoleComponent.OnGameAttach` | appelle l'original **puis** masque | ✅ interception cosmétique |
| 9 | `StealthMappinController.ShouldDisableMappin` | `true` | ✅ prédicat vanilla, usage légitime |
| 10 | `PlayerPuppet.OnGameAttached` | ancrage du système | — (plomberie) |

**Le module n'est donc pas uniformément mauvais** : 3 hooks sur 10 respectent déjà la doctrine.
La reconstruction porte sur 5 cas, pas sur l'ensemble — c'est beaucoup plus petit que « tout »,
ce que le backlog laissait craindre.

## Pourquoi le cas n°1 est le plus grave

`ScriptableDeviceComponentPS.GetActions` est l'**ancêtre commun** de toutes les PS de devices qui
n'overrident pas cette méthode. Le couper c'est couper une famille entière **sans jamais dire
laquelle**. Deux conséquences déjà payées :

- **Le bug des ascenseurs (backlog B2)** : `TerminalControllerPS.GetActions` commence par
  `if( !( super.GetActions(...) ) ) { return false; }` — il **consomme** le booléen. Notre `false`
  ne sautait donc pas seulement les actions de base, il court-circuitait toute la chaîne terminal.
  Corrigé le 2026-07-27 par une exemption étroite, vérifiée en jeu avec son contre-test.
  **Cette exemption doit disparaître avec la reconstruction** — mais pas avant.
- **`GetActions` n'est pas un entonnoir.** C'est un *producteur* d'actions, consommé par plusieurs
  appelants dont certains dépendent de sa valeur de retour. Intercepter là, c'est intercepter en
  amont de tout le monde, sans discriminer. C'est structurellement la mauvaise couche.

## Entonnoirs candidats, par famille

Le modèle est `SendLiftStartDelayedEvent` pour l'ascenseur : **un seul point que toute voie doit
franchir**, où l'on voit ce qui passe et où l'on peut rejouer l'ordre serveur.

| Famille | Ce qu'on veut vraiment | Entonnoir candidat (à sonder) |
| --- | --- | --- |
| Devices monde (1) | que le joueur ne *déclenche* pas l'effet local — pas qu'il ne voie rien | l'**exécution** de l'action (`ScriptableDeviceAction` / le chemin `ExecuteAction`), pas sa production |
| Distributeurs (2,3) | idem, plus l'inventaire côté serveur | même entonnoir que (1) : ce sont des devices |
| Tourelles (4) | que la tourelle n'agisse pas de son propre chef | probablement l'ordre de tir / l'attitude, pas le menu d'actions |
| Police (5) | que l'escalade soit décidée par le serveur | le **déclenchement d'un niveau de recherche**, pas l'existence du système |
| Hostilité (6) | déjà bon | `TryChangingAttitudeToHostile` **est** un entonnoir — à garder |

⚠️ **Aucune de ces cibles n'est mesurée.** Ce sont des candidats déduits du code décompilé, pas des
points d'entrée prouvés. La doctrine impose de **sonder avant d'implémenter** — une sonde Lua CET
par famille, comme pour l'ascenseur (`PROTOCOLE-SONDAGE.md`).

## Ordre proposé

1. **Devices (1,2,3)** — c'est la famille qui a déjà mordu, et la plus large. Sonder le chemin
   d'exécution d'une action de device et vérifier qu'il est unique.
2. **Police (5)** — le retour anticipé sur `OnAttach` est le plus brutal des cinq : il empêche un
   système entier d'exister, donc tout ce qui en dépend échoue en silence.
3. **Tourelles (4)** — cas isolé, faible surface.
4. Ne rien toucher à (6), (7), (8), (9) : ils respectent déjà la doctrine.

## Ce qui reste vrai du module actuel

- Le **levier à chaud** (`Tessera_SetLever`, `DesossageConsole.reds`) est bon et doit survivre à la
  reconstruction : il permet de rallumer une famille sans rebuild, ce qui est indispensable pour
  sonder (c'est lui qui a permis d'isoler la variable au test ascenseur du 2026-07-27).
- La **configuration par défaut** (tout coupé) et sa lecture vivante (`GetLiveConfig`) restent le
  bon modèle.
- Les commentaires du module portent une **vraie valeur documentaire** (pièges `@wrapMethod`,
  classes sans override propre, signatures vérifiées contre le script décompilé) — à reprendre,
  pas à jeter.
