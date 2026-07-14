module Tessera.UiKit.Test

// Écran #5 du florilège (spec 2026-07-14, D-H3) : console de commandes dev/debug — champ texte +
// sortie, usage fonctionnel (pas immersif).
// STUB : journalise l'intention.
//
// D-H7 : aucun équivalent natif pour une vraie console de commandes — stratégie fixée dès le
// départ = reconstruction minimale (D-U12 stratégie 2). Risque de collision de déclencheur avec
// l'écran #1 (téléphone) signalé en Partie 3 de la spec, non tranché ici. Ce stub ne fait que
// réserver le nom d'écran "devconsole" dans le dispatcher pour la suite.
public func Tessera_UiTestDevConsole(game: GameInstance, open: Bool) -> Void {
  FTLog(s"[Tessera/UiTest] devconsole → open=\(open) (STUB, reconstruction prévue — pas de recherche native, cf. D-H7)");
}
