module Tessera.Desossage

// Leviers population : piétons, trafic véhicules, transit (métro/passants).
// STUB : journalise l'intention. Corps réel (densité des communautés / trafic) à pincer en jeu.

// Recherche (dump RTTI du jeu) : GameInstance.GetCommunitySystem -> gameCommunitySystem a
// EnableDynamicCrowdNullArea(areaLocalBBox: Box, areaLocalToWorld: WorldTransform, savable: Bool,
// duration: Float) -> Uint64 (id à garder pour DisableCrowdNullArea(id) plus tard) — symbole réel,
// c'est le mécanisme même utilisé par le jeu pour supprimer le spawn de foule dans une zone. MAIS
// construire un Box/WorldTransform correct (coordonnées « grand monde ») sans vérification en jeu
// est trop risqué après l'incident du jour — PIN IN-GAME plutôt que deviner la structure exacte.
public func Tessera_ApplyPedestrians(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  let factor: Float = 0.0;
  if e.active { factor = e.density; }
  FTLog(s"[Tessera/Desossage] (stub) piétons → densité \(factor)");
}

// Recherche : GameInstance.GetTrafficSystem -> worldTrafficScriptInterface n'expose qu'une
// requête (IsPathIntersectingWithTraffic), pas de levier de densité. Rien de vérifié trouvé.
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
