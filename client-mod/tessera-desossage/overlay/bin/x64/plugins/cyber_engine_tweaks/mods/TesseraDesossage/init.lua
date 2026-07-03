-- Panneau CET pour piloter DesossageConfig sans taper de commandes.
-- Appelle Tessera_SetLever (@addMethod(PlayerPuppet), DesossageConsole.reds) — aucune
-- logique métier ici, ce fichier ne fait qu'exposer une UI.

TesseraDesossage = TesseraDesossage or {}

TesseraDesossage.levers = {
  { key = "pedestrians", label = "Piétons", note = "Réel — réagit en direct" },
  { key = "traffic", label = "Trafic véhicules", note = "Stub — aucun effet" },
  { key = "vendors", label = "Vendeurs", note = "Stub — aucun effet" },
  { key = "transit", label = "Transit (métro)", note = "Stub — aucun effet" },
  { key = "police", label = "Police", note = "Réel — BOOT ONLY, recharge nécessaire" },
  { key = "ambientSecurity", label = "Sécurité ambiante", note = "Stub — aucun effet" },
  { key = "ncpdHustles", label = "Hustles NCPD", note = "Stub — aucun effet" },
  { key = "randomEncounters", label = "Rencontres aléatoires", note = "Stub — aucun effet" },
  { key = "cyberpsychos", label = "Cyberpsychos", note = "Stub — aucun effet" },
  { key = "fastTravel", label = "Voyage rapide", note = "Réel — BOOT ONLY, recharge nécessaire" },
  { key = "vendingDevices", label = "Distributeurs", note = "Réel — BOOT ONLY, recharge nécessaire" },
  { key = "worldInteractables", label = "Interactables monde", note = "Stub — aucun effet" },
  { key = "questTriggers", label = "Appels fixers", note = "Réel (partiel) — décoché = icône radio déverrouillée" },
  { key = "tutorials", label = "Tutoriels", note = "Stub — aucun effet" },
}

TesseraDesossage.leverState = TesseraDesossage.leverState or {}
for _, lever in ipairs(TesseraDesossage.levers) do
  if TesseraDesossage.leverState[lever.key] == nil then
    TesseraDesossage.leverState[lever.key] = false
  end
end

TesseraDesossage.dayNightScale = TesseraDesossage.dayNightScale or 1.0

function TesseraDesossage:RenderLevers()
  for _, lever in ipairs(TesseraDesossage.levers) do
    local current = TesseraDesossage.leverState[lever.key]
    local newValue = ImGui.Checkbox(lever.label, current)
    if newValue ~= current then
      TesseraDesossage.leverState[lever.key] = newValue
      local density = newValue and 1.0 or 0.0
      Game.GetPlayer():Tessera_SetLever(lever.key, newValue, density)
    end
    ImGui.SameLine()
    ImGui.TextDisabled("(" .. lever.note .. ")")
  end
end

function TesseraDesossage:RenderWorld()
  ImGui.Separator()
  ImGui.Text("Monde")
  local newScale = ImGui.SliderFloat("Échelle jour/nuit", TesseraDesossage.dayNightScale, 0.0, 4.0)
  if newScale ~= TesseraDesossage.dayNightScale then
    TesseraDesossage.dayNightScale = newScale
    Game.GetPlayer():Tessera_SetLever("dayNightCycleScale", true, newScale)
  end
  ImGui.SameLine()
  ImGui.TextDisabled("(1.0 normal, 0.0 figé, stub — aucun effet confirmé)")
end

local isOverlayVisible = false

function TesseraDesossage:Render()
  if not isOverlayVisible then return end

  if ImGui.Begin('Tessera Désossage', ImGuiWindowFlags.AlwaysAutoResize) then
    if Game.GetPlayer() == nil then
      ImGui.Text("Charge une session d'abord.")
      ImGui.End()
      return
    end

    TesseraDesossage:RenderLevers()
    TesseraDesossage:RenderWorld()
  end
  ImGui.End()
end

registerForEvent('onDraw', function() TesseraDesossage:Render() end)
registerForEvent('onOverlayOpen', function() isOverlayVisible = true end)
registerForEvent('onOverlayClose', function() isOverlayVisible = false end)
