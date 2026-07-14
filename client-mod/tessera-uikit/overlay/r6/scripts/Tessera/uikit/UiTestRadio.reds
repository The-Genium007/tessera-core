module Tessera.UiKit.Test

// Écran #2 du florilège (spec 2026-07-14, D-H3) : radio véhicule (stations FM).
// STUB : journalise l'intention, la vraie ouverture reste à confirmer en jeu (palier H1).
//
// Recherche RTTI (tools/nativedb, 2026-07-14) : deux classes réelles distinctes — le HUD
// permanent en véhicule `CarRadioGameController` (parent gameuiHUDGameController, méthode
// `OnRadioChange(Bool) -> Bool`, pas un vrai "ouvrir/fermer") et surtout le **popup de
// sélection de station** `VehicleRadioPopupGameController` (parent BaseModalListPopupGameController)
// qui expose `Activate() -> Void` et `OnClose() -> Void` — signature la plus proche d'un
// ouvrir/fermer direct trouvée dans tout le florilège.
//
// PIN IN-GAME : `Activate()`/`OnClose()` sont prometteuses mais nécessitent d'abord une instance
// vivante de `VehicleRadioPopupGameController` (le joueur doit être en véhicule) — mécanisme de
// récupération d'instance non confirmé (même inconnue que pour les autres écrans HUD du
// florilège, cf. UiTestPhone.reds). À essayer au palier H1 : ouvrir la radio normalement en jeu,
// inspecter `scripting.log` pour voir si CET expose l'instance active.
public func Tessera_UiTestRadio(game: GameInstance, open: Bool) -> Void {
  FTLog(s"[Tessera/UiTest] radio → open=\(open) (STUB, voir commentaire du fichier pour les pistes RTTI)");
}
