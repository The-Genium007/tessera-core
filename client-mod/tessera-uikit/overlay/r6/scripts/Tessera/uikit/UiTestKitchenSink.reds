module Tessera.UiKit.Test

// Écran #6 du florilège (spec 2026-07-14, D-H3) : "kitchen sink" de composants (bouton, toggle,
// slider, champ texte, liste déroulante) — cartographier ce que tessera-uikit sait afficher.
// STUB : journalise l'intention, la vraie instanciation reste à confirmer en jeu (palier H1).
//
// Recherche RTTI (tools/nativedb, 2026-07-14) : contrairement aux écrans #1-#3 (qui exigent de
// retrouver une instance HUD déjà vivante), les composants ici sont des **logic controllers ink
// génériques**, pas liés à un écran précis — `inkSliderController` (parent
// inkWidgetLogicController, méthodes `ChangeValue(Float)`/`GetCurrentValue() -> Float`/
// `GetMinValue()`/`GetMaxValue()`) et `BaseToggleView` (trouvé via `search.py class Toggle`)
// sont candidats pour être attachés à des widgets qu'on crée NOUS-MÊMES (même famille que le
// pattern déjà validé pour les icônes en U1 : `SetAtlasResource`/`SetTexturePart` référencent
// l'existant sans le copier — ici on référencerait le LOGIC CONTROLLER plutôt que l'image).
// `gameuiSettingsMenuGameController` (menu Réglages complet) reste la source de référence pour
// voir COMMENT le jeu assemble ces logic controllers sur ses propres widgets, à inspecter via
// WolvenKit en local (D-U3) si l'attache directe échoue.
//
// PIN IN-GAME : instancier un `inkCanvas` à nous (comme `UiKitProbe.reds` prévu en U1), y créer
// un widget, et essayer `widget.SetLogicController(new inkSliderController())` (syntaxe exacte à
// vérifier — D-U6). Si `SetLogicController` n'existe pas sous ce nom, chercher l'équivalent dans
// un mod publié qui personnalise un slider (même méthode que pour les autres leviers désossage :
// RTTI d'abord, script décompilé ensuite, mod réel en dernier recours).
public func Tessera_UiTestKitchenSink(game: GameInstance, open: Bool) -> Void {
  FTLog(s"[Tessera/UiTest] kitchensink → open=\(open) (STUB, voir commentaire du fichier pour les pistes RTTI)");
}
