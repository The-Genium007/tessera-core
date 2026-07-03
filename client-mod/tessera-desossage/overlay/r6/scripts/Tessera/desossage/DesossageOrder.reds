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
  FTLog(s"[Tessera/Desossage] sécurité ambiante → gérée par hook SecurityTurretControllerPS.GetActions");
}

// `GetActions(out array<gamedeviceAction>, GetActionsContext) -> Bool` est déclarée sur
// `ScriptableDeviceComponentPS` (classe mère commune à TOUTES les PS de devices du jeu — vending,
// access points, NCART, tourelles…), confirmé via le dump RTTI (WopsS/RED4ext.NativeDB,
// classes/ScriptableDeviceComponentPS.json). Même pattern que `VendingMachineControllerPS`
// (cf. DesossageDevices.reds) : coupe le menu d'interaction/quickhack sur les tourelles.
// N'EMPÊCHE PAS la tourelle de détecter/tirer sur le joueur si hostile (ça, c'est le comportement
// IA de la tourelle, pas les actions joueur) — couvre la partie « interaction », pas « fonctionne
// encore comme ennemi ambiant ».
// PISTE PLUS FORTE (non implémentée, pas testable sans jeu) : `SecurityTurretControllerPS
// .SetDeviceState(EDeviceStatus)` avec `EDeviceStatus.DISABLED` (valeur -2, confirmée dans
// enums/EDeviceStatus.json) désactiverait probablement la tourelle elle-même (même mécanisme que
// le quickhack "Overload" du jeu de base). À essayer en suivant sur un hook `GameAttached` si le
// fix ci-dessous s'avère insuffisant en jeu.
// PIN IN-GAME : à confirmer (le menu doit disparaître ; la tourelle peut encore réagir aux PNJ).
@wrapMethod(SecurityTurretControllerPS)
protected func GetActions(out actions: array<ref<DeviceAction>>, context: GetActionsContext) -> Bool {
  if !DesossageConfig.Default().ambientSecurity.active {
    return false;
  }
  return wrappedMethod(actions, context);
}
