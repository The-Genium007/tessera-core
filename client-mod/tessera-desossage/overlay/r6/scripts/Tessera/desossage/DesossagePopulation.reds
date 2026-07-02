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

// Recherche : GameInstance.GetTrafficSystem -> TrafficSystem n'expose que des requêtes
// (IsPathIntersectingWithTraffic, FindEntitiesNearPlane), confirmé par le script décompilé
// officiel — pas de levier de densité dédié. HYPOTHÈSE À TESTER (console CET) : le trafic
// véhicules est peut-être aussi gouverné par CommunitySystem.ChangeDensityModifier (le « Community
// System » du jeu couvre la population ambiante en général, pas juste les piétons) — si couper
// `pedestrians` réduit aussi le trafic observé en jeu, ce stub devient inutile.
public func Tessera_ApplyTraffic(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  let factor: Float = 0.0;
  if e.active { factor = e.density; }
  // PIN IN-GAME : régler la densité du trafic véhicules à `factor`.
  FTLog(s"[Tessera/Desossage] (stub) trafic → densité \(factor)");
}

public func Tessera_ApplyTransit(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  let factor: Float = 0.0;
  if e.active { factor = e.density; }
  // PIN IN-GAME : régler la densité du transit (métro NCART / passants de transit) à `factor`.
  FTLog(s"[Tessera/Desossage] (stub) transit → densité \(factor)");
}
