module Tessera.Desossage

// Levier monde : échelle du cycle jour/nuit (le monde statique/temps/météo/son est gardé).
// STUB : journalise l'intention. Corps réel (échelle de temps du cycle) à pincer en jeu.

// 1.0 = normal ; 2.0 = journée 2x plus longue ; 0.0 = cycle figé.
public func Tessera_ApplyDayNightScale(game: GameInstance, scale: Float) -> Void {
  // PIN IN-GAME : appliquer `scale` à l'échelle de temps du cycle jour/nuit.
  FTLog(s"[Tessera/Desossage] (stub) cycle jour/nuit → échelle \(scale)");
}
