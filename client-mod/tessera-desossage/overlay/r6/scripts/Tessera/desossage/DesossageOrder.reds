module Tessera.Desossage

// Leviers ordre public : police / système de recherche (wanted/MaxTac) + sécurité ambiante.
//
// Bug constaté en jeu (build dev-7) : le hook précédent, @wrapMethod(PreventionSystem)
// CanPreventionReactToInput(), ne coupait PAS le gain d'étoiles (étoiles obtenues en tirant sur
// des PNJ malgré police.active=false). Sa source ("preventionSystem.swift") était fausse — CP2077
// n'a pas de sources Swift décompilées — et aucune trace de ce symbole n'existe dans la communauté
// modding. Root cause probable : symbole inventé, jamais réellement vérifié.
//
// Remplacé par un levier corroboré par plusieurs mods publiés (Nexus "Disable Police System" #9263,
// forums schaken-mods) : neutraliser `PreventionSystem.OnAttach()`, le point d'init de tout le
// système (heat/spawns NCPD/MaxTac). Lu directement sur `DesossageConfig.Default()` (pas via
// `DesossageSystem.Get()`) pour ne pas dépendre de l'ordre d'attache des ScriptableSystem —
// `DesossageSystem` pourrait ne pas encore être attaché quand `PreventionSystem.OnAttach` tourne.
// PIN IN-GAME : à reconfirmer sur le prochain build (log `[Tessera/Desossage]` + test tir/crime).
@wrapMethod(PreventionSystem)
private func OnAttach() -> Void {
  if !DesossageConfig.Default().police.active {
    FTLog(s"[Tessera/Desossage] police → PreventionSystem.OnAttach coupé (police.active=false)");
    return;
  }
  wrappedMethod();
}

public func Tessera_ApplyPolice(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  FTLog(s"[Tessera/Desossage] police → gérée par hook PreventionSystem.OnAttach");
}

public func Tessera_ApplyAmbientSecurity(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  if e.active {
    return;
  }
  // PIN IN-GAME : désactiver les devices de sécurité ambiants (tourelles/drones).
  FTLog(s"[Tessera/Desossage] (stub) sécurité ambiante → coupée");
}
