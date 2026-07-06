-- HUD toujours visible, indépendant de l'overlay CET (~) : GPS + shard courant/zone tampon +
-- contrôle trafic/piétons. Contrairement à TesseraDesossage/init.lua (panneau complet, gated par
-- onOverlayOpen/onOverlayClose, pensé pour du réglage ponctuel de tous les leviers), ce mod reste
-- affiché en continu pendant le jeu.
--
-- Le toggle "Trafic/piétons" ici appelle le MÊME levier "pedestrians" que TesseraDesossage
-- (Tessera_SetLever, DesossageConsole.reds) — confirmé en jeu (2026-07-05) que "traffic" est un
-- doublon de "pedestrians" (même CommunitySystem.ChangeDensityModifier, cf. DesossagePopulation.reds
-- lignes 19-40) : il n'existe pas de levier véhicule séparé côté RTTI, donc pas de compteur de
-- véhicules natif à afficher ici — seulement l'état du contrôle, l'observation visuelle reste manuelle.
--
-- Bloc shard (spec playtest-shards §#2, registre ③ "HUD debug CET") : lit shard-map.json, généré
-- côté serveur par `tessera-directory topology export --manifest <toml> --out shard-map.json`
-- (tessera-core/directory/src/shard_map.rs) à partir du manifeste de topologie — source unique de
-- vérité, pas de sync réseau (décision A de la spec). À régénérer/recopier ici si la topologie du
-- serveur change (triviale, topologie figée pour le playtest). X/Y serveur = X/Y monde du jeu
-- (GetWorldPosition brut), donc aucune transformation de repère nécessaire.

TesseraHUD = TesseraHUD or {}
TesseraHUD.trafficOn = TesseraHUD.trafficOn or false
TesseraHUD.trafficDensity = TesseraHUD.trafficDensity or 1.0
-- Masqué par défaut : ce HUD ne s'affiche qu'après activation via le hotkey CET ci-dessous
-- (Paramètres > Input > Bindings > "Tessera HUD : afficher/masquer"), pas au démarrage du jeu.
TesseraHUD.visible = false

-- Chemin absolu du dossier du mod (mods/TesseraHUD/) — une simple string relative "shard-map.json"
-- se résoudrait par rapport au cwd du jeu (bin/x64/), pas ce dossier ; technique standard CET.
local MOD_DIR = debug.getinfo(1, "S").source:match("^@(.*[/\\])") or ""

local function loadShardMap()
  local f = io.open(MOD_DIR .. "shard-map.json", "r")
  if f == nil then return nil end
  local content = f:read("*a")
  f:close()
  local ok, data = pcall(json.decode, content)
  if not ok then return nil end
  return data
end

TesseraHUD.shardMap = TesseraHUD.shardMap or loadShardMap()

local function withinBounds(v, lo, hi)
  if lo ~= nil and v < lo then return false end
  if hi ~= nil and v > hi then return false end
  return true
end

-- Résout l'id de shard contenant (x,y) + la distance à la frontière (split) la plus proche +
-- si on est dans sa zone tampon (± radius.base). nil si shard-map.json est absent/invalide.
function TesseraHUD:ComputeShardInfo(x, y)
  local map = TesseraHUD.shardMap
  if map == nil then return nil end

  local currentId = "?"
  for _, s in ipairs(map.shards) do
    if withinBounds(x, s.minX, s.maxX) and withinBounds(y, s.minY, s.maxY) then
      currentId = s.id
      break
    end
  end

  local nearestDist = nil
  for _, sp in ipairs(map.splits) do
    local coord = (sp.axis == "x") and x or y
    local d = math.abs(coord - sp.at)
    if nearestDist == nil or d < nearestDist then nearestDist = d end
  end

  local inBuffer = nearestDist ~= nil and nearestDist <= map.radius.base

  return { shardId = currentId, borderDist = nearestDist, inBuffer = inBuffer }
end

function TesseraHUD:Render()
  if not TesseraHUD.visible then return end

  local player = Game.GetPlayer()
  if player == nil then return end

  ImGui.SetNextWindowPos(20, 280, ImGuiCond.FirstUseEver)
  ImGui.SetNextWindowBgAlpha(0.55)
  ImGui.Begin("Tessera HUD", true, ImGuiWindowFlags.AlwaysAutoResize)

  local pos = player:GetWorldPosition()
  local yaw = player:GetWorldOrientation():ToEulerAngles().yaw
  ImGui.Text(string.format("X: %.1f  Y: %.1f  Z: %.1f", pos.x, pos.y, pos.z))
  ImGui.Text(string.format("Yaw: %.1f", yaw))

  ImGui.Separator()
  local shardInfo = TesseraHUD:ComputeShardInfo(pos.x, pos.y)
  if shardInfo == nil then
    ImGui.TextDisabled("(shard-map.json introuvable)")
  else
    ImGui.Text("Shard: " .. shardInfo.shardId)
    if shardInfo.borderDist ~= nil then
      ImGui.Text(string.format("Distance frontière: %.1f", shardInfo.borderDist))
    end
    if shardInfo.inBuffer then
      ImGui.TextColored(1.0, 0.7, 0.0, 1.0, "DANS ZONE TAMPON")
    end
  end

  ImGui.Separator()
  ImGui.Text("Trafic / piétons")
  local newOn = ImGui.Checkbox("Actif", TesseraHUD.trafficOn)
  if newOn ~= TesseraHUD.trafficOn then
    TesseraHUD.trafficOn = newOn
    player:Tessera_SetLever("pedestrians", newOn, TesseraHUD.trafficDensity)
  end

  local newDensity = ImGui.SliderFloat("Densité", TesseraHUD.trafficDensity, 0.0, 2.0)
  if newDensity ~= TesseraHUD.trafficDensity then
    TesseraHUD.trafficDensity = newDensity
    if TesseraHUD.trafficOn then
      player:Tessera_SetLever("pedestrians", true, newDensity)
    end
  end

  ImGui.End()
end

registerForEvent('onDraw', function() TesseraHUD:Render() end)

-- Bindable dans Paramètres > Input > Bindings (CET) — aucune touche imposée par défaut, l'utilisateur
-- choisit la sienne. registerHotkey est l'API CET standard pour ça (persistée par CET lui-même).
registerHotkey('TesseraHUD_Toggle', 'Tessera HUD : afficher/masquer', function()
  TesseraHUD.visible = not TesseraHUD.visible
end)
