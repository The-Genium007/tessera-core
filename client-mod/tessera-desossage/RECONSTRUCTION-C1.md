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
| Devices monde (1) | que le joueur ne *déclenche* pas l'effet local — pas qu'il ne voie rien | ✅ **ÉTABLI : `QueuePSDeviceEvent`** — voir ci-dessous |
| Distributeurs (2,3) | idem, plus l'inventaire côté serveur | même entonnoir que (1) : ce sont des devices |
| Tourelles (4) | que la tourelle n'agisse pas de son propre chef | 🟡 `SecurityTurretControllerPS.OnSetDeviceAttitude` — **scripté**, à sonder |
| Police (5) | que l'escalade soit décidée par le serveur | ✅ **ÉTABLI : `PreventionSystem.ChangeHeatStage`** — **scripté** |
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

## ✅ L'entonnoir des devices est établi — et il impose du C++ (2026-07-27)

**Trouvé sur le script décompilé, pas déduit.** `ExecutePSAction` / `ExecutePSActionWithDelay`
(`scriptableDeviceBasePS.script:6366+`) ne sont que des enveloppes : **41 sites** d'exécution
d'action de device convergent tous vers

```
GamePersistencySystem.QueuePSDeviceEvent(action : DeviceAction)
```

C'est le `SendLiftStartDelayedEvent` des devices : un point unique que toute voie doit franchir,
au niveau de l'**exécution** de l'effet et non de sa **production**. Il permettrait en prime de
**journaliser ce qu'on coupe** — le reproche principal fait au désossage actuel.

### ⚠️ Mais il n'est PAS atteignable en redscript

Établi par compilation (`scc`, 3 essais, chacun éliminant une cause) :

1. `@wrapMethod(gamePersistencySystem)` → `[UNRESOLVED_REF]`. Le dump RTTI dit
   `gamePersistencySystem` ; **le nom redscript est `GamePersistencySystem`**
   (`importonly final class`, `varDBSystem.script:53`). Piège classique du CLAUDE.md — le script
   décompilé fait foi pour les noms.
2. Nom corrigé → `[UNRESOLVED_METHOD]`, « signature does not match ». Retirer `final` n'y change
   rien.
3. **Cause réelle** : `QueuePSDeviceEvent` est déclarée `public **import** function` — c'est un
   **natif**. `@wrapMethod` a besoin d'un corps scripté où se greffer ; un natif n'en a pas.
   Contrôle croisé : **toutes** les cibles que le désossage wrappe avec succès sont des fonctions
   **scriptées** (`TryChangingAttitudeToHostile` = `public static function`, `ShouldDisableMappin`
   = `private function`, tous les `GetActions`…). Aucune n'est `import`.

### Conséquence sur la forme du chantier

Le volet **devices de C1 est un chantier RED4ext, pas redscript.** Le patron existe déjà dans
`tessera-core/client-mod/tessera-client` : `UniversalRelocFunc` + hash AddressLib +
`hooking->Attach`, avec la phase 1 log-only (c'est ainsi que `ExecuteNode` et
`StreamingSector::PostLoad` ont été abordés).

Ordre révisé : commencer par le **hook natif log-only** sur `QueuePSDeviceEvent` (mesurer le débit
et le contenu des actions **avant** toute décision de blocage — leçon du canal de fractures), puis
seulement ensuite réinjecter l'autorité serveur.

> Une sonde redscript avait été écrite pour ce point puis **supprimée** : elle ne compile pas, et
> un `.reds` fautif fait tomber tout `r6/scripts`. Le constat qu'elle a produit est ci-dessus.

## ✅ Volet devices — phase 1 IMPLÉMENTÉE (2026-07-27)

Hook natif écrit et **compilé** dans `tools/re-probe` : `TesseraRE_FunnelWatch(on)` arme
l'observation de `QueuePSDeviceEvent`. **Log-only, désarmé par défaut, ne bloque jamais.**

**Sans aucune adresse en dur** : le hash AddressLib (`3797228879`, RVA `0x561768`) vient de la
table livrée par le jeu — voir `tools/reverse-engineering/RVA-VERS-HASH-ADDRESSLIB.md`, méthode
découverte le même jour et qui rend hookable n'importe quelle fonction repérée dans Ghidra.

Chiffres qui ont tranché entre redscript et C++ : **109** appels passent par `ExecutePSAction`
(scriptée, wrappable) mais **41** appellent le natif en direct — ~73 % de couverture. La doctrine
interdit une voie non gardée, donc redscript était disqualifié quoi qu'il arrive.

**Reste à faire** (in-game, une session) : armer, mesurer le **débit** et les **classes d'action**
qui passent, vérifier que toutes les interactions de device y apparaissent. Ce n'est qu'après
qu'on décide de la forme du blocage — c'est le raccourci inverse qui a produit le désossage actuel.

## Familles 4 et 5 — entonnoirs identifiés le 2026-07-27, et ils sont SCRIPTÉS

Bonne nouvelle après le volet devices (qui, lui, impose du C++) : ces deux-là restent en redscript.

### Police (5) — `PreventionSystem.ChangeHeatStage(newHeatStage, heatChangeReason)`

**Seul site d'affectation de `m_heatStage`** dans tout le système (vérifié : une seule occurrence
de `m_heatStage = `). Toute montée ou descente de niveau de recherche y passe. Le niveau lui-même
n'est qu'un **fait de quête** (`wanted_level`), écrit par `SetWantedLevelFact` en aval.

Trois raisons d'en faire l'entonnoir plutôt que l'actuel retour anticipé sur `OnAttach` :

- **le système continue d'exister** — donc tout ce qui en dépend (radio police, barre de recherche,
  télémétrie, sirènes) ne casse plus en silence ;
- la fonction porte **`heatChangeReason`**, une chaîne : on sait *pourquoi* ça monte, donc on peut
  journaliser ce qu'on refuse — impossible aujourd'hui ;
- c'est le point exact où réinjecter l'autorité serveur (le serveur décide de l'escalade, le client
  la joue), au lieu de désactiver la police pour tout le monde.

`private function`, mais `@wrapMethod` sait wrapper du privé (le désossage le fait déjà sur
`ShouldDisableMappin`, `private final func`).

### Tourelles (4) — `SecurityTurretControllerPS.OnSetDeviceAttitude(evt)`

⚠️ **À sonder, pas encore prouvé.** Le hook actuel (`GetActions` → `false`) porte sur le **menu
d'interaction du joueur**, pas sur le fait que la tourelle agisse — ce n'est donc même pas la bonne
chose qui est coupée. Le point de décision est l'attitude du device
(`protected export override function OnSetDeviceAttitude`, scripté).

À vérifier en sonde : les tourelles ambiantes sont-elles hostiles **par défaut** (auquel cas
l'attitude n'est jamais « posée » et l'entonnoir serait ailleurs, du côté de l'état initial) ?

## Bilan du cadrage C1

| Famille | Entonnoir | Techno | État |
| --- | --- | --- | --- |
| Devices (1,2,3) | `QueuePSDeviceEvent` | **C++ RED4ext** | ✅ hook phase 1 écrit et compilé |
| Police (5) | `PreventionSystem.ChangeHeatStage` | redscript | ✅ identifié, à sonder |
| Tourelles (4) | `OnSetDeviceAttitude` | redscript | 🟡 candidat, à sonder |
| Hostilité (6) | `TryChangingAttitudeToHostile` | redscript | ✅ déjà bon, à garder |
| (7)(8)(9) | — | — | ✅ conformes à la doctrine |

**Les 5 cas à reprendre ont tous leur entonnoir identifié.** Un seul exige du C++. Le chantier est
passé de « tout est à refaire » à une liste finie avec sa technologie déterminée pour chaque ligne.
