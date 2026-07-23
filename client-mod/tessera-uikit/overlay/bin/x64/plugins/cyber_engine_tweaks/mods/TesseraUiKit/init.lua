-- Hotkey CET → ouvre le panneau démo UIKit H2 (redscript, popup Codeware InGamePopup).
-- Le panneau se ferme avec ESC (géré nativement par le popup). Ce mod CET ne fait QUE binder une
-- touche et appeler le pont redscript Tessera_ShowUiKitDemo — toute l'UI vit côté redscript.
--
-- Bind : overlay CET (touche ~ par défaut) > onglet "Bindings"/"Hotkeys" > "Tessera UiKit : panneau
-- démo". Aucune touche imposée par défaut (l'utilisateur choisit la sienne), même convention que
-- TesseraHUD.

print("[TesseraUiKit] init.lua chargé — hotkey en cours d'enregistrement")

registerHotkey("TesseraUiKitDemo", "Tessera UiKit : panneau démo", function()
  local player = Game.GetPlayer()
  if player == nil then
    print("[TesseraUiKit] pas de joueur (menu principal ?) — entre en jeu d'abord")
    return
  end
  print("[TesseraUiKit] hotkey pressé — appel de Tessera_ShowUiKitDemo")
  player:Tessera_ShowUiKitDemo()
end)

-- Lobby d'arrivée v1 (UiKitLobby.reds) — écran chronologiquement premier du parcours joueur.
registerHotkey("TesseraUiKitLobby", "Tessera UiKit : lobby d'arrivée", function()
  local player = Game.GetPlayer()
  if player == nil then
    print("[TesseraUiKit] pas de joueur (menu principal ?) — entre en jeu d'abord")
    return
  end
  print("[TesseraUiKit] hotkey pressé — appel de Tessera_ShowLobby")
  player:Tessera_ShowLobby()
end)

print("[TesseraUiKit] hotkeys enregistrés")
