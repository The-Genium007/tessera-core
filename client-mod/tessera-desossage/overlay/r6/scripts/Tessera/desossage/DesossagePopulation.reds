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
