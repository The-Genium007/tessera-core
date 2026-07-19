module Tessera.UiKit.Test

// Écran #1 du florilège (spec 2026-07-14, D-H3) : téléphone de V (appels/SMS).
// STUB : journalise l'intention, la vraie ouverture reste à confirmer en jeu (palier H1).
//
// Recherche RTTI (tools/nativedb, 2026-07-14) : l'écran réel est `HudPhoneGameController`
// (parent gameuiProjectedHUDGameController, champ `.PhoneSystem: whandle:PhoneSystem`,
// `.CurrentFunction: EHudPhoneFunction`) — élément HUD toujours instancié, pas une fenêtre à
// créer. `PhoneSystem` (gameScriptableSystem) expose `IsPhoneAvailable() -> Bool` et
// `OnUsePhone(handle:UsePhoneRequest) -> Void` mais ce dernier attend un objet requête construit,
// pas un simple booléen. Le seul accesseur déjà confirmé sur ce projet est
// `GameInstance.GetPhoneManager(game).ApplyPhoneCallRestriction(Bool)` (désossage `phoneCalls`,
// ex-`questTriggers`, DesossageEvents.reds) — bloque/débloque les appels, n'ouvre pas l'écran lui-même.
//
// PIN IN-GAME : deux pistes à essayer en priorité au palier H1 (console CET, sans rebuild) —
// (a) `GameInstance.GetPhoneManager(game)` expose peut-être une méthode d'ouverture directe non
// listée ici (RTTI incomplet côté générateur du dump, à vérifier avec `search.py show` si une
// classe plus précise apparaît côté jeu) ; (b) construire/poster un `PickupPhoneRequest` ou
// `UsePhoneRequest` via le système de requêtes du joueur, pattern déjà vu ailleurs dans le RTTI
// (`PickupPhoneRequest`, `TalkingTriggerRequest`) mais jamais exercé sur ce projet.
public func Tessera_UiTestPhone(game: GameInstance, open: Bool) -> Void {
  FTLog(s"[Tessera/UiTest] phone → open=\(open) (STUB, voir commentaire du fichier pour les pistes RTTI)");
}
