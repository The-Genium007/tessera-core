module Tessera.Desossage

// Leviers ordre public : police / système de recherche (wanted/MaxTac) + sécurité ambiante.
// STUB : journalise l'intention. Corps réel (PreventionSystem, devices sécurité) à pincer en jeu.

public func Tessera_ApplyPolice(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  if e.active {
    FTLog(s"[Tessera/Desossage] (stub) police active — non coupée");
    return;
  }
  // PIN IN-GAME : désactiver PreventionSystem (heat 0, pas de spawns NCPD/MaxTac).
  FTLog(s"[Tessera/Desossage] (stub) police/prévention → coupée");
}

public func Tessera_ApplyAmbientSecurity(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  if e.active {
    return;
  }
  // PIN IN-GAME : désactiver les devices de sécurité ambiants (tourelles/drones).
  FTLog(s"[Tessera/Desossage] (stub) sécurité ambiante → coupée");
}
