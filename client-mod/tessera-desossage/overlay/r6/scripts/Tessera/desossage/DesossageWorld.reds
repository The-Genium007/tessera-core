module Tessera.Desossage

// Levier monde : échelle du cycle jour/nuit (le monde statique/temps/météo/son est gardé).
// STUB : journalise l'intention. Corps réel (échelle de temps du cycle) à pincer en jeu.

// 1.0 = normal ; 2.0 = journée 2x plus longue ; 0.0 = cycle figé.
// Recherche (dump RTTI du jeu) : gameTimeSystem.SetTimeDilation(reason, dilation, ...) existe
// mais c'est un ralenti GLOBAL (façon bullet-time/Sandevistan) qui affecterait AUSSI le
// mouvement/combat du joueur — contraire à la contrainte de design (« le joueur... gardé »,
// seuls temps/météo doivent varier). Pas de multiplicateur dédié cycle-jour/nuit-seul trouvé
// dans l'API de gameTimeSystem. PIN IN-GAME : creuser weatherSystem ou un multiplicateur dédié.
public func Tessera_ApplyDayNightScale(game: GameInstance, scale: Float) -> Void {
  FTLog(s"[Tessera/Desossage] (stub) cycle jour/nuit → échelle \(scale)");
}
