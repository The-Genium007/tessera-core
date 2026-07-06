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

local function loadShardMap()
  -- `pcall(json.decode, content)` évaluerait `json.decode` comme argument AVANT l'appel de
  -- pcall — si `json` n'existe pas, l'erreur se produit hors de la protection de pcall et fait
  -- planter le chargement du mod entier. Vérifier `json` d'abord rend ce chemin sûr dans tous
  -- les cas (que `json` existe ou non pour les mods CET).
  if type(json) ~= "table" or type(json.decode) ~= "function" then return nil end

  -- Chemin relatif nu : CET résout déjà tout chemin io/dofile par rapport au dossier du mod
  -- lui-même (mods/TesseraHUD/), pas au cwd du jeu — confirmé par la doc CET ("all pathing is
  -- now relative to mods"). CORRIGÉ (2026-07-06) : la version précédente calculait elle-même un
  -- chemin absolu via `debug.getinfo(1, "S")`, qui plantait au tout premier chargement du fichier
  -- (avant même l'enregistrement du hotkey) — confirmé en jeu via la console CET ("TesseraHUD a
  -- une erreur de chargement", aucun des autres mods Tessera n'étant affecté). `debug.getinfo`
  -- n'est apparemment pas fiable/disponible dans ce sandbox de mod ; supprimé plutôt que retenté.
  local f = io.open("shard-map.json", "r")
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

-- Radar 2D top-down (aligné sur les axes monde, ne tourne PAS avec le regard du joueur — plus
-- simple/fiable qu'un radar orienté). Portée fixe autour du joueur ; la frontière de shard et sa
-- zone tampon sont dessinées si elles tombent dans cette portée. Alternative choisie (2026-07-06)
-- à des balises 3D dans le monde (nécessiterait de faire spawn des entités via redscript, plus
-- risqué) ou des mappins carte (API RegisterMappin difficile à wrapper, cf. spec playtest-shards).
-- PIN IN-GAME : API de dessin ImGui (GetWindowDrawList/AddLine/AddRectFilled/GetColorU32) confirmée
-- exister par la doc CET (binding quasi 1:1 avec Dear ImGui) mais jamais testée dans CE jeu — les
-- signatures exactes (valeurs de retour multiples vs tables Vector2) sont ma meilleure hypothèse,
-- à corriger si le rendu est incorrect/plante.
local RADAR_SIZE = 140
local RADAR_RANGE_M = 100.0

function TesseraHUD:RenderRadar(pos)
  local map = TesseraHUD.shardMap
  if map == nil then return end

  local scale = (RADAR_SIZE / 2) / RADAR_RANGE_M
  ImGui.Text(string.format("Radar (portée %.0fm)", RADAR_RANGE_M))

  local drawList = ImGui.GetWindowDrawList()
  local originX, originY = ImGui.GetCursorScreenPos()
  local cx, cy = originX + RADAR_SIZE / 2, originY + RADAR_SIZE / 2

  drawList:AddRectFilled(originX, originY, originX + RADAR_SIZE, originY + RADAR_SIZE, ImGui.GetColorU32(0.0, 0.0, 0.0, 0.35))

  for _, sp in ipairs(map.splits) do
    local axisIsX = sp.axis == "x"
    local playerCoord = axisIsX and pos.x or pos.y
    local offset = (sp.at - playerCoord) * scale
    if math.abs(offset) <= RADAR_SIZE / 2 then
      local bufferPx = (map.radius.base or 0) * scale
      local bufferColor = ImGui.GetColorU32(1.0, 0.7, 0.0, 0.25)
      local lineColor = ImGui.GetColorU32(1.0, 0.3, 0.3, 1.0)
      if axisIsX then
        drawList:AddRectFilled(cx + offset - bufferPx, originY, cx + offset + bufferPx, originY + RADAR_SIZE, bufferColor)
        drawList:AddLine(cx + offset, originY, cx + offset, originY + RADAR_SIZE, lineColor, 2.0)
      else
        drawList:AddRectFilled(originX, cy + offset - bufferPx, originX + RADAR_SIZE, cy + offset + bufferPx, bufferColor)
        drawList:AddLine(originX, cy + offset, originX + RADAR_SIZE, cy + offset, lineColor, 2.0)
      end
    end
  end

  -- Joueur au centre, dessiné en dernier pour rester au-dessus de la frontière/zone tampon.
  drawList:AddCircleFilled(cx, cy, 4, ImGui.GetColorU32(0.2, 0.8, 1.0, 1.0))

  ImGui.Dummy(RADAR_SIZE, RADAR_SIZE) -- réserve l'espace du dessin dans le layout ImGui
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
  TesseraHUD:RenderRadar(pos)

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
