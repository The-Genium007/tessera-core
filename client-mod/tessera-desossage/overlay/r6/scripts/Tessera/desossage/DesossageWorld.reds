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

// Saut direct à une heure précise — `gameTimeSystem.SetGameTimeByHMS(h, m, s, reason)`, confirmé
// par le script décompilé officiel (CDPR-Modding-Documentation/Cyberpunk-Scripts,
// scripts/core/systems/timeSystem.script) ET plusieurs mods publiés réels qui l'appellent
// (CyanideX/NovaCityTools, MaximiliumM/appearancemenumod, Avi6481/EasyTrainers — tous via
// `Game.GetTimeSystem():SetGameTimeByHMS(...)`). Contrairement à MappinSystem, `gameTimeSystem`
// GARDE son nom RTTI complet côté redscript (pas de préfixe à retirer ici).
// N'affecte QUE l'horloge/l'éclairage — pas de ralenti joueur/combat, contrairement à
// SetTimeDilation (raison pour laquelle dayNightCycleScale, lui, reste un stub).
// PIN IN-GAME : jamais testé sur cette machine avant ce build.
public func Tessera_DoJumpToTime(game: GameInstance, hour: Int32, minute: Int32) -> Void {
  GameInstance.GetTimeSystem(game).SetGameTimeByHMS(hour, minute, 0, n"tessera_desossage");
  FTLog(s"[Tessera/Desossage] heure → \(hour)h\(minute)");
}
