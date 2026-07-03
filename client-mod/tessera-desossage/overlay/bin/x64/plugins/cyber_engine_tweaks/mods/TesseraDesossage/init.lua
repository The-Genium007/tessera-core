-- Panneau CET pour piloter DesossageConfig sans taper de commandes.
-- Appelle Tessera_SetLever (@addMethod(PlayerPuppet), DesossageConsole.reds) — aucune
-- logique métier ici, ce fichier ne fait qu'exposer une UI.

TesseraDesossage = TesseraDesossage or {}

local isOverlayVisible = false

function TesseraDesossage:Render()
  if not isOverlayVisible then return end

  if ImGui.Begin('Tessera Désossage', ImGuiWindowFlags.AlwaysAutoResize) then
    if Game.GetPlayer() == nil then
      ImGui.Text("Charge une session d'abord.")
      ImGui.End()
      return
    end

    ImGui.Text("Squelette OK — leviers à venir.")
  end
  ImGui.End()
end

registerForEvent('onDraw', function() TesseraDesossage:Render() end)
registerForEvent('onOverlayOpen', function() isOverlayVisible = true end)
registerForEvent('onOverlayClose', function() isOverlayVisible = false end)
