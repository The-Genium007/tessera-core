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
// d'hérité d'utile). Confirme et durcit la suspicion précédente (qui citait aussi
// FindEntitiesNearPlane — absent du dump, probablement une confusion). HYPOTHÈSE TOUJOURS À TESTER
// (console CET) : le trafic véhicules est peut-être aussi gouverné par
// CommunitySystem.ChangeDensityModifier (le « Community System » du jeu couvre la population
// ambiante en général, pas juste les piétons) — si couper `pedestrians` réduit aussi le trafic
// observé en jeu, ce stub devient inutile. Reste un stub tant que non testé.
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
