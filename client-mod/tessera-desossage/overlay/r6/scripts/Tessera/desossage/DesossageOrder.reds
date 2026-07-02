module Tessera.Desossage

// Leviers ordre public : police / système de recherche (wanted/MaxTac) + sécurité ambiante.
// Symboles 2.31 confirmés via source décompilée (core/systems/preventionSystem.swift).

public func Tessera_ApplyPolice(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  if e.active {
    return;
  }
  // Le PreventionSystem lit le fact `prevention_quest_disabled` (== 1 → il se désactive :
  // pas de heat/NCPD/MaxTac). Cf. preventionSystem.swift (GetFact("prevention_quest_disabled")).
  GameInstance.GetQuestsSystem(game).SetFact(n"prevention_quest_disabled", 1);
  FTLog(s"[Tessera/Desossage] police/prévention → coupée (prevention_quest_disabled=1)");
}

public func Tessera_ApplyAmbientSecurity(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  if e.active {
    return;
  }
  // PIN IN-GAME : désactiver les devices de sécurité ambiants (tourelles/drones).
  FTLog(s"[Tessera/Desossage] (stub) sécurité ambiante → coupée");
}
