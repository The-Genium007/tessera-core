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
| Tourelles (4) | que la tourelle n'agisse pas de son propre chef | ✅ `SensorDevice.SetAsIntrestingTarget` — **scripté**, sonde écrite et compilée |
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

### Tourelles (4) — `SensorDevice.SetAsIntrestingTarget(target)`

**`OnSetDeviceAttitude` est écarté.** C'était le candidat inscrit ici ; la lecture du script
décompilé le réfute. Ce n'est pas un entonnoir mais **une entrée parmi d'autres** — celle des
*surcharges* (quickhack, programme de piratage, forçage de quête). Sur
`SecurityTurretControllerPS` il ne fait rien d'autre que notifier
(`securityTurretController.script:643` → `super` → `Notify`).

L'hostilité d'une tourelle **ambiante** ne passe jamais par là : elle vient du système d'attitude
(`IsAttitudeFromContextHostile`, `sensorDeviceController.script:828`, qui interroge
l'`AttitudeAgent`). Hooker `OnSetDeviceAttitude` n'aurait donc coupé que les surcharges du joueur,
en laissant la tourelle tirer exactement comme avant — **même classe d'erreur que le hook
`GetActions` actuel**, qui coupe le *menu d'interaction* et pas le *comportement*. Deux fois le
même piège sur la même famille : le point qui *ressemble* à une décision n'est pas celui qui décide.

Le vrai point de décision est `SensorDevice.SetAsIntrestingTarget(target) -> Bool`
(`sensorDevice.script:857`) : « cet objet est-il une cible pour moi ? ». Deux branches, qui
couvrent les deux régimes :

- relié à un système de sécurité **et** attitude non modifiée → `SecuritySystem.ShouldReactToTarget` ;
- sinon → `GetAttitudeTowards(this, target) == AIA_Hostile`.

C'est bien un entonnoir : **deux appelants seulement**, tous deux dans `SensorDevice` — détection
d'un nouvel objet (`:1435`) et ré-évaluation des cibles courantes (`ReevaluateTargets`, `:2484`).
Rien ne devient une cible sans passer là.

**Portée du hook** : posé sur `SensorDevice` (la base), pas sur `SecurityTurret`. L'override de la
tourelle (`securityTurret.script:296`) est un **passe-plat pur** (`return super.…`), donc tout
retombe dans la base de toute façon — et on capte en prime les **caméras**, même famille, même
décision, une seule sonde.

Sonde : `tools/re-probe/overlay/r6/scripts/Tessera/reprobe/SensorTargetProbe.reds`, log-only
(verdict rendu tel quel, aucun retour anticipé), compilée `scc` sans erreur. Elle ne journalise que
les appels **visant le joueur local** : `ReevaluateTargets` boucle sur toutes les cibles et peut
être relancée souvent — une sonde qui fait chuter le framerate ne mesure plus rien, et c'est de
toute façon la seule décision qu'on voudra arbitrer côté serveur.

## Bilan du cadrage C1

| Famille | Entonnoir | Techno | État |
| --- | --- | --- | --- |
| Devices (1,2,3) | `QueuePSDeviceEvent` | **C++ RED4ext** | ✅ hook phase 1 écrit et compilé |
| Police (5) | `PreventionSystem.ChangeHeatStage` | redscript | ✅ identifié, à sonder |
| Tourelles (4) | `SensorDevice.SetAsIntrestingTarget` | redscript | ✅ identifié, sonde écrite et compilée |
| Hostilité (6) | `TryChangingAttitudeToHostile` | redscript | ✅ déjà bon, à garder |
| (7)(8)(9) | — | — | ✅ conformes à la doctrine |

**Les 5 cas à reprendre ont tous leur entonnoir identifié.** Un seul exige du C++. Le chantier est
passé de « tout est à refaire » à une liste finie avec sa technologie déterminée pour chaque ligne.

## Session de mesure du 2026-07-28 — ce qui a été établi, et ce qui ne l'a pas été

### Mesure brute

Hook C1 armé (`TesseraRE_FunnelWatch(true)`), joueur **immobile** au point de chargement,
`survey_area` : 61 entités dans 60 m — 41 objets statiques, 14 véhicules, 6 PNJ, **aucun device,
aucun ascenseur**. Résultat : **0 action en 120 s**, en trois échantillons.

**Ce zéro ne mesure rien.** Sans contrôle positif il ne distingue pas « l'entonnoir est froid » de
« le hook ne se déclenche jamais ». Il est *cohérent* avec un joueur immobile sans device autour —
mais la cohérence n'est pas une preuve. À ne pas reporter comme un débit.

### Condition expérimentale (à consigner au même instant que la mesure, sinon échantillons non comparables)

**Le désossage n'était PAS déployé** — `r6/scripts/Tessera/` ne contient que `elevators`, `reprobe`,
`uikit`. La mesure a donc porté sur du **vanilla nu**, ce qui est la bonne base de référence pour
C1 : on veut le débit de l'entonnoir tel que le jeu le produit, avant toute coupure.

J'ai d'abord attribué ce zéro au désossage. C'était faux, et vérifiable en une commande (lister le
dossier de scripts). **Vérifier ce qui est réellement chargé avant d'expliquer un résultat** — une
explication plausible et non vérifiée coûte plus cher qu'un « je ne sais pas ».

### Le verrou réel, lui, tient — mais pour la PROCHAINE mesure

L'analyse du code reste valide et mordra dès que le désossage sera réinstallé :

| Famille | Ce que coupe le désossage | Conséquence sur la mesure |
| --- | --- | --- |
| Devices | `worldInteractables` inactif → `GetActions` rend `false` (sauf ascenseurs) | aucune action n'est créée, donc **rien n'atteint jamais** `QueuePSDeviceEvent` |
| Police | `police` inactif → retour anticipé sur `PreventionSystem.OnAttach` | le système n'existe pas, donc `ChangeHeatStage` **n'est jamais appelée** |

> **Le désossage détruit précisément ce qu'il faut mesurer pour le remplacer.** C'est la boucle
> fermée du raccourci d'origine : on a coupé sans mesurer, et la coupure interdit maintenant la
> mesure. Corollaire à généraliser : **tout entonnoir situé en aval d'une coupure existante est
> aveugle tant que la coupure tient** — remplacer un hook de désossage commence toujours par
> l'éteindre.

**Levée du verrou** : action `lever` ajoutée au pont du harnais (pilote `Tessera_SetLever`, qui
n'existait qu'en frappe manuelle dans la console CET, donc non scriptable). Elle a été vérifiée
négativement dans cette session — `Tessera_SetLever` est `nil` quand le désossage n'est pas
déployé, et le pont le dit en clair au lieu d'échouer en silence.

### Le hook est prouvé sur la bonne fonction (vérification statique, 2026-07-28)

Un zéro n'a de sens que si l'instrument est le bon. Chaîne de preuve, fermée des deux bouts :

1. Ghidra — la chaîne `"QueuePSDeviceEvent"` (`0x142bc2f08`) n'a **qu'une** référence, depuis
   `FUN_140e44ee4`, qui est l'**enregistrement RTTI** de ce nom (motif `_Init_thread_header` +
   `atexit`, typique d'un enregistrement statique). Elle lie le natif à `FUN_1405616ec`.
2. `FUN_1405616ec` appelle `FUN_140561768` (`0x14056174c`, `UNCONDITIONAL_CALL`) — la fonction
   hookée. Le thunk déballe les arguments, le corps fait le travail.
3. Le corps décompilé confirme la signature : `(param_1, longlong* param_2)` où `param_2` est un
   `Handle<T>` (**incrément atomique du compteur de références** sur `param_2[1]`), suivi de la
   lecture de 16 octets à `+0x40` (un `PersistentID`) et d'un pointeur à `+0x50`. C'est exactement
   `QueuePSDeviceEvent(system, Handle<DeviceAction>&)`.
4. Table AddressLib livrée par le jeu : hash **3797228879 ↔ RVA 0x561768**, aller-retour vérifié
   dans les deux sens. (Le thunk RTTI `0x5616ec` a un hash distinct, 353182615 — aucune confusion
   possible entre les deux.)

Le corps est le bon point d'attache : il capte tout ce qui atteint l'implémentation, là où le thunk
ne verrait que les appels venus du script.

**Donc le 0 mesuré signifie « l'entonnoir était froid », pas « le hook est faux ».** C'est ce qui
rend la suite interprétable.

### Protocole de mesure (ordre imposé)

1. si le désossage est déployé : `lever|worldInteractables on 1.0` et `lever|police on 0.0` ;
2. confirmer dans le log jeu (`[Tessera/Desossage] SetLever`) — un nom de levier inconnu est refusé
   **en silence** côté redscript, le retour du pont ne prouve rien ;
3. `funnel_watch|on`, **jouer**, échantillonner le compteur ;
4. `funnel_watch|off`, dépouiller `tesserareprobe-*.log` et les lignes `[Tessera/C1/Police]`.

⚠️ **L'étape 3 exige un joueur humain.** Les deux entonnoirs ne s'ouvrent que sur du jeu réel
(interagir avec un device, se faire repérer) — aucune sonde ne les alimente à vide, et le point de
chargement de la sauvegarde d'essai n'a aucun device à portée. C'est la limite honnête de ce
chantier : **la phase 1 de C1 n'est pas automatisable**.

## ✅ MESURE DU 2026-07-29 — volet devices

Enfin du jeu réel, sur l'install vanilla (désossage NON déployé, Cyberverse absent — donc aucune
coupure, l'entonnoir peut s'ouvrir). **41 actions sur 155 s.**

### Débit — et c'est lui qui débloque la phase 2

| mesure | valeur |
| --- | --- |
| débit moyen | **0,26 action/s** |
| plus grosse rafale | 5 (chargement de session) |
| plus grosse rafale d'origine joueur | 4, étalées sur 267 ms |
| rafale typique | 2 actions en ~12 ms |

**Le serveur peut arbitrer chaque action device individuellement.** On est plus d'un ordre de
grandeur sous le plafond au-delà duquel une décision synchrone devient intenable (~4/s, seuil
mesuré sur le canal de fractures). C'était LA question ouverte de la phase 1 : elle est tranchée,
et dans le bon sens.

### Les 8 classes vues

| classe | occurrences | origine |
| --- | --- | --- |
| `ForceUnlockAndOpenElevator` | 14 | cycle de portes de cabine |
| `ForceLockElevator` | 14 | idem — **toujours appariée** à la précédente |
| `SetDeviceOFF` | 5 | chargement de session, aucune action du joueur |
| `VehicleDoorInteraction` | 2 | portière de véhicule |
| `QuestForceDisabled` | 2 | système de quêtes, aucune action du joueur |
| `DispenceItemFromVendor` | 2 | distributeur (⚠️ voir plus bas) |
| `ToggleOpen` | 1 | joueur |
| `OpenVendorUI` | 1 | joueur |

### Trois enseignements

1. **Les deux actions d'ascenseur vont par paire**, à ~12 ms d'écart, 14 fois sur 14. Le verrou et
   l'ouverture sont un seul geste du moteur : les intercepter séparément serait une erreur de
   granularité.
2. **`VehicleDoorInteraction` passe par cet entonnoir.** Les portières de véhicule sont des actions
   de device PS — un seul point d'interception couvre donc devices ET portières, ce qui simplifie
   C3 le jour où on y viendra.
3. **Le streaming et les quêtes empruntent le même tuyau que le joueur** (`SetDeviceOFF` au
   chargement, `QuestForceDisabled` en cours de partie, aucun des deux provoqué). C'est la
   démonstration expérimentale que `GetActions → false` était la mauvaise réponse : la phase 2 doit
   discriminer **l'origine** de l'action, pas couper le tuyau. Un blocage indiscriminé casserait
   des quêtes, en silence.

### ✅ Point tranché — un achat émet DEUX fois

La première trace montrait `DispenceItemFromVendor` ×2, mais trois gestes s'y mélangeaient. **Test
contrôlé refait à une seule variable** (un achat au distributeur, rien d'autre — pas de boutique,
pas d'ascenseur au retour) :

```
action #18 : DispenceItemFromVendor   21:32:01.820
action #19 : DispenceItemFromVendor   21:32:01.934      (écart : 114 ms)
```

**Deux émissions pour un achat, et rien d'autre.** `OpenVendorUI` reste à 0 — cette classe-là venait
bien d'un autre geste (ouverture d'une boutique de vente), ce qui confirme au passage que
l'interface d'un distributeur ne l'émet pas.

Écart mesuré deux fois indépendamment : **109 ms** puis **114 ms**. Cohérent, donc structurel — ce
n'est pas une double-frappe de l'utilisateur.

**Conséquence pour la phase 2, et elle est concrète :** le jour où l'argent sera autoritatif, un
serveur qui débiterait sur chaque action de vendeur **facturerait deux fois chaque achat**. La
déduplication n'est pas une optimisation, c'est une correction obligatoire — fenêtre de l'ordre de
200 ms sur le couple (device, item), à confirmer sur d'autres types de distributeurs.

C'est exactement le genre de détail qui ne coûte rien à mesurer maintenant et très cher à
découvrir en playtest, sur un joueur qui se plaint d'avoir payé double.

## ✅ MESURE DU 2026-07-29 — volet police

Une poursuite complète, montée puis redescente. **Deux événements. C'est tout.**

```
ChangeHeatStage -> niveau=1  raison=EnterCombat
ChangeHeatStage -> niveau=0  raison=SystemTimeOut
```

### Ce que ça tranche

**Les deux sens passent bien par le même point.** C'était LA question du volet : la montée
(`preventionSystem.script:2532`, qui incrémente de +1) et la redescente à zéro (`:4795`) appellent
toutes deux `ChangeHeatStage`. Un troisième chemin, le forçage par quête (`:3998`), y passe aussi.
L'entonnoir est donc complet — aucune voie de secours à découvrir après coup.

**Le débit est dérisoire** : 2 événements pour une poursuite entière. Là où les devices demandaient
une vraie mesure pour savoir si un arbitrage serveur synchrone tenait, ici la question ne se pose
même pas. Le serveur peut décider de chaque changement de niveau sans y penser.

**L'escalade se fait cran par cran** (`+1` à chaque appel, `:2531`) : passer de 0 à 3 étoiles
produira 3 événements distincts, pas un saut direct. Le serveur voit donc chaque palier.

### Les raisons — 2 observées sur ~12 possibles

Relevées dans le script décompilé, et classées par ce qui compte pour la phase 2 : **qui décide**.

| `heatChangeReason` | origine | observée |
| --- | --- | --- |
| `EnterCombat` | acte du joueur | ✅ |
| `CrimeWitness` | acte du joueur, vu par un témoin | — |
| `Kill` / `KillPrevention` | acte du joueur | — |
| `SystemTimeOut` | automatique, décrue | ✅ |
| `SecurityAreaReset` | automatique, zone | — |
| `QuestEvent` / `QuestWantedLevel` / `QuestPreventionTriggerSoftDeescalation` / `Preset` | scénario | — |
| `ResetOnPlayerChoice` | choix de dialogue | — |
| `DEBUG` | outillage | — |

La coupure utile saute aux yeux : les raisons **« acte du joueur »** sont celles que le serveur doit
arbitrer (c'est lui qui sait si le tir a touché, si un PNJ a vu) ; les raisons **automatiques** et
**scénario** peuvent rester locales tant qu'elles produisent le même résultat chez tous. Cette
taxonomie ne demandait pas la mesure — mais savoir que `EnterCombat` est bien la porte d'entrée
réelle du cas courant, si.

### Reste ouvert

Le joueur n'a atteint que le **niveau 1** : l'escalade multi-paliers n'a pas été observée, seulement
déduite du script (`+1` par appel). À confirmer d'une session où la traque monte à 3-4 étoiles —
faible enjeu, le code est explicite, mais c'est la différence entre déduit et mesuré.

Note : le canal devices n'a **pas bougé** pendant toute la poursuite (41 avant, 41 après). Une
course-poursuite ne génère aucun trafic de device.

## ⏸️ Volet capteurs — DIFFÉRÉ le 2026-07-29 (non bloquant)

Pas mesuré : aucune tourelle alimentée à portée pendant la session. Deux résultats quand même,
tous deux issus du script décompilé, et qui cadrent le volet pour la prochaine fois.

**La sonde couvre trois familles, pas deux.** `SensorDevice` est hérité par `SurveillanceCamera`,
`SecurityTurret` **et `SniperNest`** — cette troisième n'était pas au cadrage. Les caméras sont de
loin les plus nombreuses : c'est par elles qu'il faudra remplir ce volet, pas par les tourelles.

**Un drone volant ne passera JAMAIS par cette sonde**, et ce n'est pas un défaut. Un drone est un
`ScriptedPuppet` portant un `DroneComponent`, pas un device : il décide de cibler par l'IA des PNJ.
Conséquence de cadrage : la décision « qui me considère comme une cible » est **scindée en deux
mécanismes** dans le jeu — capteurs fixes d'un côté, pantins de l'autre. La famille C1 « tourelles »
ne couvre que le premier ; le second relève de l'hostilité PNJ, déjà pourvue de son entonnoir
(`TryChangingAttitudeToHostile`, classé « déjà bon, à garder »). Les deux moitiés sont couvertes,
mais par deux points distincts — à ne pas confondre en phase 2.

**Contre-témoin obtenu gratuitement** : viser une tourelle ÉTEINTE n'a rien produit. C'est le
comportement correct — `TurnOffDevice()` appelle `TurnOffSenseComponent()`, donc un capteur hors
tension ne détecte rien et `SetAsIntrestingTarget` n'est jamais appelée. Si la sonde avait parlé,
c'est mon hook ou ma compréhension du mécanisme qui aurait été faux.

**Rappel de méthode pour la prochaine session** : la sonde se déclenche quand le capteur DÉTECTE le
joueur, pas quand le joueur regarde le capteur. Il faut entrer dans le champ d'un device alimenté ;
où pointe le réticule n'entre nulle part dans la décision.

## Phase 2 — ce que les mesures permettent de concevoir, et ce qui manque encore

### Le verrou conceptuel, nommé

La mesure a montré que **le streaming et les quêtes empruntent le même entonnoir que le joueur**
(`SetDeviceOFF` au chargement, `QuestForceDisabled` en cours de partie, aucun des deux provoqué).
Un blocage indiscriminé au point d'entrée casserait donc des quêtes — en silence, et sans qu'aucun
test ne le voie. C'est la démonstration **expérimentale** que `GetActions -> false` était la
mauvaise réponse : il ne suffit pas de déplacer la coupure vers un meilleur endroit, il faut la
rendre **sélective**.

### La bonne nouvelle : l'action porte son initiateur

`ScriptableDeviceAction` déclare `m_executor` (le `GameObject` qui a initié) et `m_requesterID`,
avec leurs accesseurs `GetExecutor()` / `GetRequesterID()`. Les chemins joueur les posent
explicitement — `SetExecutor(GetPlayer(...))`, `SetExecutor(context.processInitiatorObject)`,
`SetExecutor(instigator)`. L'information nécessaire à la discrimination **existe déjà sur l'objet
qui traverse l'entonnoir** : rien à reconstruire, rien à corréler.

### Pourquoi le hook reste en C++ malgré tout

`ExecutePSAction` (`scriptableDeviceBasePS.script:6366`) semblait un point d'interception plus
confortable — redscript, exécuteur sous la main. **Écarté : 109 appelants**, plusieurs surcharges,
et surtout ce n'est PAS le point unique — le harnais Lua atteint déjà `QueuePSDeviceEvent`
directement, sans passer par lui. Une garde qui laisse une voie de contournement n'est pas une
garde. Le natif reste le seul vrai goulot.

### Ce qui manque, et comment on l'obtient

**Question ouverte, unique, et qui décide de tout** : les actions du streaming et des quêtes
ont-elles un exécuteur **nul ou différent du joueur** ?

- si **oui**, la discrimination est triviale et toute la phase 2 se conçoit dessus ;
- si **non** (toutes portent le joueur), il faut un autre critère — et il vaut mieux le savoir
  avant d'avoir écrit la moindre ligne de blocage.

La sonde a été modifiée pour répondre : elle journalise désormais la classe de l'exécuteur à côté
de la classe d'action, lue par le RTTI (`m_executor` est un champ scripté, sans offset stable
qu'on puisse graver — et la recherche par nom est négligeable à 0,26 action/s). `<null>` et
`<absent>` sont distingués à dessein : « l'action n'est pas une `ScriptableDeviceAction` » n'est
pas la même information que « le champ est là mais vide ».

**Une seule session in-game suffira** : rejouer exactement le même parcours (chargement, quelques
devices, un ascenseur, un achat) et lire la colonne exécuteur. Aucune nouvelle sonde à écrire.

⚠️ Toujours pas de code bloquant, et ce n'est pas de la prudence de principe : tant que cette
question n'est pas tranchée, on ne sait pas si la garde envisagée est seulement **réalisable**.

## ✅ 2026-07-29 — LA DISCRIMINATION MARCHE, et elle sépare intention et conséquence

Question posée en fin de journée : les actions non provoquées portent-elles un exécuteur différent
du joueur ? **Oui, et la coupure est plus propre que prévu.**

| classe d'action | exécuteur | ce que c'est |
| --- | --- | --- |
| `GoToFloor` | **`PlayerPuppet`** | le joueur appuie sur un étage |
| `DispenceItemFromVendor` | **`PlayerPuppet`** | le joueur achète |
| `ForceLockElevator` (×9) | `<null>` | conséquence : le moteur verrouille |
| `ForceUnlockAndOpenElevator` (×8) | `<null>` | conséquence : le moteur ouvre |
| `SetOpened` (×2) | `<null>` | conséquence : une porte s'ouvre |

**Ce n'est pas « joueur contre système », c'est « intention contre conséquence ».** Le joueur
demande *un étage* ; le moteur en déduit verrouillage, ouverture, portes — et ces dérivées portent
un exécuteur nul. Une seule action porte l'intention, toutes les autres en découlent.

C'est exactement la coupure dont la phase 2 a besoin, et elle tombe **au bon endroit** : le serveur
arbitre l'**intention** (« ce joueur a-t-il le droit d'appeler cet étage ? ») et laisse couler les
**conséquences**, qui sont le travail du moteur — précisément la doctrine « intercepter, garder le
moteur, réinjecter l'autorité » de l'ADR 0011, retrouvée ici sur le fil plutôt que déduite.

Corollaire : la garde envisagée est **réalisable**. `executor != null` est le critère, et il est
mesuré, pas supposé.

### Le contrôle positif a fait son travail — deux fois

`DispenceItemFromVendor` était intégré au parcours parce que **je connaissais sa réponse** : un
achat est forcément d'origine joueur. Sans lui, deux sessions se seraient conclues à tort :

1. `<absent>` partout — `CClass::GetProperty` ne remonte PAS l'héritage, or `m_executor` est
   déclaré sur la classe parente. Un résultat parfaitement uniforme, donc parfaitement crédible ;
2. `<absent>` encore — **le RTTI retire le préfixe `m_`** : le champ s'appelle `executor`, pas
   `m_executor`. Trouvé par le diagnostic auto-listant les propriétés, ajouté après le premier
   échec précisément parce qu'une sonde muette n'apprend rien.

Piège à retenir, symétrique d'un piège déjà payé : **le dump RTTI ne fait pas foi sur les noms de
CLASSE** (il ignore les alias d'import) **et le script décompilé ne fait pas foi sur les noms de
PROPRIÉTÉ** (il garde le `m_` que le RTTI supprime). Les deux sources sont partielles, dans des
directions opposées.

### Limites de cette mesure

- **5 classes observées** sur les 9 connues. `QuestForceDisabled` et `SetDeviceOFF` (streaming)
  n'ont pas été rejouées cette session : l'hypothèse « elles aussi portent `<null>` » est
  plausible mais **pas mesurée**. À confirmer avant de s'y fier.
- `DispenceItemFromVendor` ×4 = **deux achats**, cohérent avec la double émission déjà établie.
