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

> **État (2026-07-02) :** leviers **réels** — police (`PreventionSystem.OnAttach`, confirmé en jeu :
> plus d'étoiles), voyage rapide (`ManageFastTravelLock`, vérifié contre le script décompilé
> officiel), piétons (`CommunitySystem.ChangeDensityModifier`, vérifié contre le script décompilé
> officiel), distributeurs boissons/nourriture (`@wrapMethod(VendingMachineControllerPS) GetActions`,
> signature vérifiée contre un mod publié réel qui wrap la même classe), déclencheurs de quêtes/
> appels fixers (`questPhoneManager.ApplyPhoneCallRestriction`, partiel — bloque les appels, pas
> les déclencheurs de zone/PNJ). **Stubs confirmés insuffisants/absents** (pas juste non-cherchés —
> vérifié contre le script décompilé officiel du jeu) — trafic (aucun levier de densité sur
> `TrafficSystem`), tutoriels (`questTutorialManager` ne ferme qu'un overlay déjà ouvert), hustles
> NCPD (aucune classe « Hustle »/« CrimeSpawn » n'existe dans le jeu, confirmé absent). **Stubs
> avec piste mais pas assez sûrs pour coder à l'aveugle** — cycle jour/nuit
> (`gameTimeSystem.SetTimeDilation` existe mais affecte aussi le joueur, mauvaise sémantique),
> vendeurs nommés, distributeurs d'armes/droppoints (classes PS sœurs de celle déjà faite, pas
> encore câblées), sécurité ambiante (`SecurityTurretControllerPS.ActionProgramSetDeviceOff`
> existe mais c'est une action quickhack par-instance, pas un switch global). **Encore aucune piste
> trouvée** — transit, rencontres aléatoires, cyberpsychos, interactables monde.
>
> **Hypothèse à tester en jeu (console CET)** : `pedestrians` (densité communautaire) pourrait
> naturellement réduire le trafic véhicules et les vendeurs ambiants s'ils spawnent via le même
> système communautaire — pas besoin de lever dédié si confirmé.
>
> **Leçon du jour :** un symbole plausible mais non vérifié (`CanPreventionReactToInput`, jamais
> confirmé) a cassé le jeu entier (crash au lancement, plusieurs cycles de réinstallation
> Windows). Toute future implémentation doit citer une source réelle (mod publié, dump RTTI —
> `nativedb.red4ext.com` / `github.com/WopsS/RED4ext.NativeDB`) — sinon rester en stub.

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
