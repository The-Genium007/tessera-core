module Tessera.Desossage

// Leviers population : piétons, trafic véhicules, transit (métro/passants).
// STUB : journalise l'intention. Corps réel (densité des communautés / trafic) à pincer en jeu.

// Réel : CommunitySystem.ChangeDensityModifier(Float) / ResetDensityModifier() — confirmé par le
// script décompilé du jeu (CDPR-Modding-Documentation/Cyberpunk-Scripts, scripts/core/systems/
// communitySystem.script). Multiplicateur global de densité des foules ambiantes — 0.0 = vide,
// 1.0 = normal. (Note : EnableDynamicCrowdNullArea existe aussi sur cette classe pour des zones
// précises, mais ChangeDensityModifier est le levier global recherché ici, et beaucoup plus
// simple/sûr — pas de Box/WorldTransform à construire.)
public func Tessera_ApplyPedestrians(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  let factor: Float = 0.0;
  if e.active { factor = e.density; }
  GameInstance.GetCommunitySystem(game).ChangeDensityModifier(factor);
  FTLog(s"[Tessera/Desossage] piétons → densité \(factor) (ChangeDensityModifier)");
}

// Repeuplement observé en jeu (bug rapporté 2026-07-06) : la densité coupée revient après
// quelques minutes, surtout en changeant de quartier. Recherché (2026-07-06, sources réelles :
// dépôt de scripts décompilés adamsmasher/cyberpunk — ChangeDensityModifier n'est déclarée que
// native, JAMAIS appelée par aucun script du jeu ; mod Nexus "No Crowds and Cars" #248, qui
// documente devoir "marcher/conduire vers une nouvelle zone" pour que son propre réglage
// s'applique) : la densité de population est réinitialisée par le moteur À CHAQUE secteur streamé
// (pas un timer, pas un reset lié au combat/à l'heure) — `ChangeDensityModifier` ne modifie que
// les communautés déjà chargées, les nouvelles reçoivent les valeurs par défaut du moteur. Pas de
// native pour "couper" ce comportement (pas de ResetDensityModifier ni équivalent trouvé) : on le
// traite en réactif plutôt qu'en préventif pur — réappliquer IMMÉDIATEMENT à l'entrée d'un
// nouveau quartier, avant que les PNJ/véhicules par défaut n'aient le temps de spawn/pop visible.
// `PreventionSystem.OnDistrictAreaEntered(handle:gamemappinsDistrictEnteredEvent)` confirmé dans
// notre propre dump RTTI (tools/nativedb) comme point d'entrée à chaque changement de quartier.
// PIN IN-GAME : visibilité exacte (`protected`) non confirmée par le RTTI (qui ne donne pas la
// visibilité) — choisie par analogie avec SecurityTurretControllerPS.GetActions
// (DesossageOrder.reds), à corriger si la compilation échoue. À valider aussi : le délai entre
// l'événement et le premier spawn est-il suffisant pour éviter tout pop-in visible ?
//
// PISTE ALTERNATIVE PLUS ROBUSTE (non implémentée) : l'API CET `GameOptions` (hors RTTI, binding
// CET pur — absent du dump car non exposé au moteur de réflexion du jeu) permettrait de modifier
// directement les réglages moteur `[Crowd]`/`[Traffic]` (engine/config/platform/pc/*.ini) que le
// streaming relit à CHAQUE secteur — plus besoin de réagir à quoi que ce soit. Mods réels utilisant
// cette voie : "Disabled Crowd" #175, "Realistic Traffic Density" #6457 (ini statiques),
// "CP77 Ini Tweaker" #15973 (même réglage via GameOptions à la volée). Clés ini exactes non
// confirmées ici (pages Nexus non accessibles en fetch direct) — à reprendre si le hook réactif
// ci-dessous s'avère insuffisant (pop-in visible malgré tout).
@wrapMethod(PreventionSystem)
protected func OnDistrictAreaEntered(evt: ref<gamemappinsDistrictEnteredEvent>) -> Void {
  wrappedMethod(evt);
  Tessera_ApplyPedestrians(GetGameInstance(), DesossageSystem.GetLiveConfig(GetGameInstance()).pedestrians);
}

// Recherche confirmée via dump RTTI complet (WopsS/RED4ext.NativeDB) : GameInstance.GetTrafficSystem
// retourne `worldTrafficScriptInterface`, qui n'expose qu'UNE seule méthode dans tout le jeu :
// IsPathIntersectingWithTraffic (requête, pas de setter de densité). Parent = IScriptable (rien
// d'hérité d'utile). AITrafficMovementSystem (la classe interne derrière l'interface scriptée) est
// tout aussi vide — zéro méthode/champ propre à aucun niveau de sa chaîne de parenté. Piste
// "système de trafic dédié" définitivement épuisée côté RTTI.
//
// HYPOTHÈSE RENFORCÉE (2026-07-03, re-creusée) : `worldPopulationSpawnerNode` (le nœud de spawn
// placé dans le monde par le level design) a un champ `.isVehicle: Bool` — piétons ET véhicules
// sont donc le MÊME type de nœud de spawn, juste distingués par ce booléen, pas deux systèmes
// séparés. Les classes `populationModifier`/`populationSpawnModifier`/
// `populationPopulationSpawnParameter` qui gravitent autour sont de purs conteneurs de données
// (zéro méthode), cohérent avec un système consommé en interne par CommunitySystem plutôt qu'une
// API scriptable parallèle. Ça renforce nettement l'hypothèse que `ChangeDensityModifier` pilote
// aussi le trafic, sans qu'on ait trouvé de setter dédié séparé pour les véhicules.
//
// CONFIRMÉ EN JEU (2026-07-05, cf. tools/nativedb/findings.md) : comparaison avant/après au même
// endroit — `pedestrians` décoché = 0 véhicule sur la rue ; `pedestrians` coché (rien d'autre
// changé) = plusieurs véhicules apparaissent. `ChangeDensityModifier` pilote donc bien les deux.
// Ce stub est un DOUBLON, pas un système à implémenter — candidat à retirer (lever + case UI dans
// TesseraDesossage/init.lua) dans un prochain nettoyage, laissé en l'état pour ne pas changer le
// schéma DesossageConfig pendant la session de test.
public func Tessera_ApplyTraffic(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  let factor: Float = 0.0;
  if e.active { factor = e.density; }
  // PIN IN-GAME : régler la densité du trafic véhicules à `factor`.
  FTLog(s"[Tessera/Desossage] (stub) trafic → densité \(factor)");
}

// Recherche (dump RTTI complet) : seule classe pertinente trouvée pour "métro/NCART" est
// `NcartTimetableControllerPS` (PS de device, même famille que VendingMachineControllerPS) — mais
// ses méthodes propres (GetCurrentTimeToDepart, ResetTimeToDepart, UpdateCurrentTimeToDepart...)
// pilotent l'affichage du panneau d'horaires en station, pas le spawn/densité de passagers ou de
// PNJ de transit. Piste écartée pour ce levier précis. Aucune autre classe "Metro"/"Transit"
// pertinente dans les 14 094 classes RTTI. PIN IN-GAME : reste à chercher côté script décompilé
// (peut-être un tag de spawn communautaire dédié plutôt qu'un système à part, comme pour
// ncpdHustles/randomEncounters — cf. DesossageEvents.reds).
public func Tessera_ApplyTransit(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  let factor: Float = 0.0;
  if e.active { factor = e.density; }
  // PIN IN-GAME : régler la densité du transit (métro NCART / passants de transit) à `factor`.
  FTLog(s"[Tessera/Desossage] (stub) transit → densité \(factor)");
}
