module Tessera.Desossage

// Leviers population : piétons, trafic véhicules, transit (métro/passants).
// STUB : journalise l'intention. Corps réel (densité des communautés / trafic) à pincer en jeu.

public func Tessera_ApplyPedestrians(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  let factor: Float = 0.0;
  if e.active { factor = e.density; }
  // PIN IN-GAME : régler le multiplicateur de densité des foules ambiantes à `factor` (0.0 = vide).
  FTLog(s"[Tessera/Desossage] (stub) piétons → densité \(factor)");
}

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
