module Tessera.UiKit

// ─────────────────────────────────────────────────────────────────────────────
// Pont CET (Lua) → panneau démo H2. Deux besoins :
//   1) obtenir un `inkGameController` VIVANT à passer comme "requester" au popup Codeware ;
//   2) déclencher l'ouverture depuis un hotkey CET (convention Tessera : TesseraHUD, désossage…).
//
// (1) Le popup a besoin d'un contrôleur seulement pour résoudre la GameInstance et poster son
// évènement sur l'UISystem. On capture le contrôleur de menu in-game — toujours présent en jeu —
// via le MÊME hook que le mod démo InkPlayground de Codeware : `RegisterInputListenersForPlayer`
// (signature confirmée sur gameuiInGameMenuGameController dans le dump RTTI : `(gameObject) -> Void`).
// On NE vole aucune touche : on stocke juste `this` sur le PlayerPuppet, et c'est un hotkey CET qui
// ouvre le panneau. Le hook se déclenche à chaque (ré)installation des listeners du joueur ; si le
// contrôleur n'a pas encore été capturé, ouvrir/fermer le menu pause une fois l'installe.
//
// (2) Pont Lua↔redscript déjà éprouvé sur ce projet (Tessera_SetLever) : `@addMethod(PlayerPuppet)`
// appelable en Lua via `Game.GetPlayer():Tessera_ShowUiKitDemo()`.
// ─────────────────────────────────────────────────────────────────────────────

// Contrôleur de menu in-game capté au dernier enregistrement des listeners — sert de requester.
@addField(PlayerPuppet)
public let m_tesseraUiKitController: wref<inkGameController>;

@wrapMethod(gameuiInGameMenuGameController)
private final func RegisterInputListenersForPlayer(playerPuppet: ref<GameObject>) -> Void {
  wrappedMethod(playerPuppet);
  let pp: ref<PlayerPuppet> = playerPuppet as PlayerPuppet;
  if IsDefined(pp) {
    pp.m_tesseraUiKitController = this;
  }
}

// Appelée depuis CET. Ouvre le panneau démo si un contrôleur a été capturé.
@addMethod(PlayerPuppet)
public func Tessera_ShowUiKitDemo() -> Void {
  if IsDefined(this.m_tesseraUiKitController) {
    TesseraUiKitDemoPopup.Show(this.m_tesseraUiKitController);
    FTLog("[Tessera/UiKit] panneau démo ouvert");
  } else {
    FTLog("[Tessera/UiKit] contrôleur non capturé — ouvre/ferme le menu pause une fois, puis réessaie");
  }
}

// Appelée depuis CET. Ouvre le LOBBY D'ARRIVÉE v1 (UiKitLobby.reds) — même mécanisme de requester.
@addMethod(PlayerPuppet)
public func Tessera_ShowLobby() -> Void {
  if IsDefined(this.m_tesseraUiKitController) {
    TesseraLobbyPopup.Show(this.m_tesseraUiKitController);
    FTLog("[Tessera/UiKit] lobby d'arrivée ouvert");
  } else {
    FTLog("[Tessera/UiKit] contrôleur non capturé — ouvre/ferme le menu pause une fois, puis réessaie");
  }
}
