module Tessera.UiKit.Test

// Écran #3 du florilège (spec 2026-07-14, D-H3) : menu radial imbriqué + émotes.
// STUB : journalise l'intention, la vraie ouverture reste à confirmer en jeu (palier H1).
//
// Recherche RTTI (tools/nativedb, 2026-07-14) : `RadialMenuGameController` (parent
// gameuiHUDGameController, le quickhack wheel) expose **`SetVisible(Bool) -> Void`** — l'appel
// le plus direct trouvé dans tout le florilège, plus `OnOpenWheelRequest`/`OnCloseWheelRequest`
// (déclenchés normalement par les events `QuickSlotButtonHoldStartEvent`/`...EndEvent`, donc pas
// utilisables tels quels depuis CET sans construire ces events). Les classes `RadialSlot` et
// `CyclableRadialSlot` existent déjà dans le RTTI — signe que le jeu gère nativement des slots
// qui changent/cyclent, donc l'hypothèse D-H5 (réutiliser le déplié natif plutôt que reconstruire
// un radial imbriqué from scratch) reste la piste prioritaire à vérifier en premier.
//
// PIN IN-GAME : même inconnue que les autres écrans HUD — récupérer une instance vivante de
// `RadialMenuGameController` pour appeler `SetVisible(true)` dessus. Si trouvée, tester ensuite
// si `CyclableRadialSlot`/`RadialSlot` supportent une hiérarchie à plusieurs niveaux (besoin
// D-H3 : jusqu'à ~20 items, sous-menus) ou si une reconstruction (stratégie 2) devient nécessaire
// pour ça spécifiquement.
public func Tessera_UiTestRadial(game: GameInstance, open: Bool) -> Void {
  FTLog(s"[Tessera/UiTest] radial → open=\(open) (STUB, voir commentaire du fichier pour les pistes RTTI)");
}
