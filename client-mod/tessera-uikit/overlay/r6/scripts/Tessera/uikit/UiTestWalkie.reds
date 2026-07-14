module Tessera.UiKit.Test

// Écran #4 du florilège (spec 2026-07-14, D-H3) : walkie-talkie / canal RP — PAS un écran, une
// touche push-to-talk + un petit indicateur HUD ("en train de parler sur canal X").
// STUB : journalise l'intention.
//
// D-H7 : aucun équivalent natif (concept 100% Tessera) — stratégie fixée dès le départ =
// reconstruction (D-U12 stratégie 2), pas de recherche RTTI native à faire ici. Le vrai
// contenu (inkText/inkCanvas overlay, câblage sur la touche push-to-talk, VoIP Mumble D9) est
// hors périmètre de cette passe (Partie 8 de la spec) — ce stub ne fait que réserver le nom
// d'écran "walkie" dans le dispatcher pour la suite.
public func Tessera_UiTestWalkie(game: GameInstance, open: Bool) -> Void {
  FTLog(s"[Tessera/UiTest] walkie → open=\(open) (STUB, reconstruction prévue — pas de recherche native, cf. D-H7)");
}
