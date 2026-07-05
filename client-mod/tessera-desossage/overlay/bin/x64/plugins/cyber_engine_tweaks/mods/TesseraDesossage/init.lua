-- Panneau CET pour piloter DesossageConfig sans taper de commandes.
-- Appelle Tessera_SetLever (@addMethod(PlayerPuppet), DesossageConsole.reds) — aucune
-- logique métier ici, ce fichier ne fait qu'exposer une UI.

TesseraDesossage = TesseraDesossage or {}

TesseraDesossage.levers = {
  { key = "pedestrians", label = "Piétons", note = "Réel — réagit en direct" },
  { key = "traffic", label = "Trafic véhicules", note = "Stub — doublon confirmé de Piétons (2026-07-05), candidat à retirer" },
  { key = "vendors", label = "Vendeurs", note = "Partiel (2026-07-05, PIN IN-GAME) — masque l'icône de rôle PNJ (GameplayRoleComponent), pas l'interaction" },
  { key = "transit", label = "Transit (métro)", note = "Stub — aucun effet" },
  { key = "police", label = "Police", note = "Réel — BOOT ONLY, recharge nécessaire" },
  { key = "ambientSecurity", label = "Sécurité ambiante", note = "Réel (menu tourelles) — probable BOOT ONLY, à confirmer" },
  { key = "gangHostility", label = "Hostilité gangs", note = "Nouveau (2026-07-05, PIN IN-GAME) — décoché = relations d'attitude neutres, gangs non hostiles" },
  { key = "ncpdHustles", label = "Hustles NCPD", note = "Stub — absent du RTTI, aucun effet" },
  { key = "randomEncounters", label = "Rencontres aléatoires", note = "Stub — absent du RTTI, aucun effet" },
  { key = "cyberpsychos", label = "Cyberpsychos", note = "Stub — absent du RTTI, aucun effet" },
  { key = "fastTravel", label = "Voyage rapide", note = "Réel — BOOT ONLY, recharge nécessaire" },
  { key = "vendingDevices", label = "Distributeurs", note = "Réel — BOOT ONLY, recharge nécessaire" },
  { key = "worldInteractables", label = "Interactables monde", note = "Réel — confirmé en jeu 2026-07-05 (drop point), couvre aussi les points d'accès" },
  { key = "questTriggers", label = "Appels fixers", note = "Réel (partiel) — décoché = icône radio déverrouillée" },
  { key = "tutorials", label = "Tutoriels", note = "Stub — aucun effet" },
  { key = "mapMarkers", label = "Marqueurs carte", note = "Nouveau (2026-07-05, PIN IN-GAME) — décoché = nettoyage PONCTUEL (recocher/décocher pour relancer), pas de blocage persistant" },
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

  -- Saut direct à une heure précise (Tessera_JumpToTime, DesossageConsole.reds) — distinct de
  -- l'échelle ci-dessus, jamais testé avant ce build (2026-07-05, PIN IN-GAME).
  if ImGui.Button("Midi (12h00)") then
    Game.GetPlayer():Tessera_JumpToTime(12, 0)
  end
  ImGui.SameLine()
  if ImGui.Button("Minuit (00h00)") then
    Game.GetPlayer():Tessera_JumpToTime(0, 0)
  end
  ImGui.SameLine()
  ImGui.TextDisabled("(saut direct, n'affecte pas le joueur/combat — PIN IN-GAME)")
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
