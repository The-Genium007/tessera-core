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
-- vérité, pas de sync réseau (décision A de la spec). X/Y serveur = X/Y monde du jeu
-- (GetWorldPosition brut), donc aucune transformation de repère nécessaire.
--
-- RÉGÉNÉRATION (ne JAMAIS éditer ce JSON à la main — piège "artefact assemblé à la main =
-- artefact qui dérive", CLAUDE.md) : depuis la racine du dépôt, contre le manifeste DÉPLOYÉ,
--   cargo run -p directory --bin tessera-directory -- topology export \
--     --manifest tessera-core/server/server.docker.toml \
--     --out tessera-core/client-mod/.../TesseraHUD/shard-map.json
-- server.docker.toml porte server_count=10 → 10 cellules Voronoï (boundaryRings + name). L'ancien
-- format BSP (minX/maxX/splits, 2 shards) avait dérivé et plantait ComputeShardInfo sur ipairs(nil)
-- (playtest 2026-07-17 : le HUD n'affichait que X/Y/Z, jamais le shard). Régénérer après tout
-- changement de topologie serveur.

TesseraHUD = TesseraHUD or {}
TesseraHUD.trafficOn = TesseraHUD.trafficOn or false
TesseraHUD.trafficDensity = TesseraHUD.trafficDensity or 1.0
-- Masqué par défaut : ce HUD ne s'affiche qu'après activation via le hotkey CET ci-dessous
-- (Paramètres > Input > Bindings > "Tessera HUD : afficher/masquer"), pas au démarrage du jeu.
TesseraHUD.visible = false

-- Log explicite à chaque étape clé (2026-07-07, demandé) : jusqu'ici, en cas de souci, la seule
-- trace était "le mod a une erreur de chargement" dans la console CET, sans dire QUOI — impossible
-- de diagnostiquer à distance sans deviner. print() va dans la console/log CET, toujours safe.
print("[TesseraHUD] fichier init.lua en cours de chargement...")

local function loadShardMap()
  -- `pcall(json.decode, content)` évaluerait `json.decode` comme argument AVANT l'appel de
  -- pcall — si `json` n'existe pas, l'erreur se produit hors de la protection de pcall et fait
  -- planter le chargement du mod entier. Vérifier `json` d'abord rend ce chemin sûr dans tous
  -- les cas (que `json` existe ou non pour les mods CET).
  if type(json) ~= "table" or type(json.decode) ~= "function" then
    print("[TesseraHUD] json indisponible dans ce sandbox CET — shard-map désactivé")
    return nil
  end

  -- Chemin relatif nu : CET résout déjà tout chemin io/dofile par rapport au dossier du mod
  -- lui-même (mods/TesseraHUD/), pas au cwd du jeu — confirmé par la doc CET ("all pathing is
  -- now relative to mods"). CORRIGÉ (2026-07-06) : la version précédente calculait elle-même un
  -- chemin absolu via `debug.getinfo(1, "S")`, qui plantait au tout premier chargement du fichier
  -- (avant même l'enregistrement du hotkey) — confirmé en jeu via la console CET ("TesseraHUD a
  -- une erreur de chargement", aucun des autres mods Tessera n'étant affecté). `debug.getinfo`
  -- n'est apparemment pas fiable/disponible dans ce sandbox de mod ; supprimé plutôt que retenté.
  local f = io.open("shard-map.json", "r")
  if f == nil then
    print("[TesseraHUD] shard-map.json introuvable (chemin relatif au dossier du mod)")
    return nil
  end
  local content = f:read("*a")
  f:close()
  local ok, data = pcall(json.decode, content)
  if not ok then
    print("[TesseraHUD] shard-map.json trouvé mais JSON invalide : " .. tostring(data))
    return nil
  end
  print("[TesseraHUD] shard-map.json chargé (" .. #(data.shards or {}) .. " shards)")
  return data
end

TesseraHUD.shardMap = TesseraHUD.shardMap or loadShardMap()

print("[TesseraHUD] init.lua chargé jusqu'au bout — hotkey en cours d'enregistrement")

-- Schéma Voronoï (shard_map.rs) : chaque shard est un id logique ("group-N") + `boundaryRings`,
-- des anneaux polygonaux (pas de `minX/maxX` ni de `splits` axis-aligned — ce schéma a remplacé
-- l'ancien format BSP le 2026-07-09, cf. Groupe G ; ce fichier n'avait pas suivi jusqu'ici et
-- plantait sur `map.splits`/`ipairs(nil)` dès qu'un vrai shard-map.json post-Voronoï était chargé,
-- trouvé le 2026-07-14). Point-in-polygon (ray casting) remplace les comparaisons de bornes.
local function pointInRing(x, y, ring)
  local inside = false
  local n = #ring
  local j = n
  for i = 1, n do
    local xi, yi = ring[i][1], ring[i][2]
    local xj, yj = ring[j][1], ring[j][2]
    if ((yi > y) ~= (yj > y)) and (x < (xj - xi) * (y - yi) / (yj - yi) + xi) then
      inside = not inside
    end
    j = i
  end
  return inside
end

local function pointInShard(x, y, shard)
  for _, ring in ipairs(shard.boundaryRings) do
    if pointInRing(x, y, ring) then return true end
  end
  return false
end

local function distToSegment(px, py, ax, ay, bx, by)
  local dx, dy = bx - ax, by - ay
  local lenSq = dx * dx + dy * dy
  if lenSq < 1e-9 then
    return math.sqrt((px - ax) ^ 2 + (py - ay) ^ 2)
  end
  local t = ((px - ax) * dx + (py - ay) * dy) / lenSq
  t = math.max(0, math.min(1, t))
  local cx, cy = ax + t * dx, ay + t * dy
  return math.sqrt((px - cx) ^ 2 + (py - cy) ^ 2)
end

-- Distance à l'arête la plus proche du polygone du shard courant : chaque arête est partagée
-- avec la cellule Voronoï voisine, donc "distance à ma propre frontière" équivaut à "distance à
-- la frontière la plus proche" — pas besoin d'itérer les shards voisins.
local function nearestEdgeDist(x, y, shard)
  local nearest = nil
  for _, ring in ipairs(shard.boundaryRings) do
    local n = #ring
    for i = 1, n do
      local a = ring[i]
      local b = ring[(i % n) + 1]
      local d = distToSegment(x, y, a[1], a[2], b[1], b[2])
      if nearest == nil or d < nearest then nearest = d end
    end
  end
  return nearest
end

-- Résout le shard contenant (x,y) + la distance à son arête de frontière la plus proche + si on
-- est dans sa zone tampon (± radius.base). nil si shard-map.json est absent/invalide.
function TesseraHUD:ComputeShardInfo(x, y)
  local map = TesseraHUD.shardMap
  if map == nil then return nil end

  local currentShard = nil
  for _, s in ipairs(map.shards) do
    if pointInShard(x, y, s) then
      currentShard = s
      break
    end
  end

  if currentShard == nil then
    return { shardId = "?", shardName = "?", borderDist = nil, inBuffer = false }
  end

  local borderDist = nearestEdgeDist(x, y, currentShard)
  local inBuffer = borderDist ~= nil and borderDist <= (map.radius.base or 0)

  -- `name` est le libellé humain émis par le générateur (repli sur l'id côté serveur si aucun
  -- quartier nommé). Repli Lua supplémentaire sur `id` pour rester robuste à un ancien shard-map
  -- sans champ `name` (compat ascendante — jamais de nil affiché).
  return {
    shardId = currentShard.id,
    shardName = currentShard.name or currentShard.id,
    borderDist = borderDist,
    inBuffer = inBuffer,
  }
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

-- Frontières polygonales Voronoï (pas de ligne de split axis-aligned unique à dessiner à travers
-- tout le cadre) : dessine les arêtes du polygone du shard COURANT qui tombent dans la portée du
-- radar, plus un cercle de zone tampon de rayon `radius.base` autour du joueur (approximation
-- simple d'une distance-à-arête uniforme — une bande par segment suivrait l'angle exact de
-- chaque arête mais ajoute une complexité géométrique non nécessaire pour un HUD de debug).
function TesseraHUD:RenderRadar(pos)
  local map = TesseraHUD.shardMap
  if map == nil then return end

  local scale = (RADAR_SIZE / 2) / RADAR_RANGE_M
  ImGui.Text(string.format("Radar (portée %.0fm)", RADAR_RANGE_M))

  local drawList = ImGui.GetWindowDrawList()
  local originX, originY = ImGui.GetCursorScreenPos()
  local cx, cy = originX + RADAR_SIZE / 2, originY + RADAR_SIZE / 2

  drawList:AddRectFilled(originX, originY, originX + RADAR_SIZE, originY + RADAR_SIZE, ImGui.GetColorU32(0.0, 0.0, 0.0, 0.35))

  local shardInfo = TesseraHUD:ComputeShardInfo(pos.x, pos.y)
  local currentShard = nil
  if shardInfo ~= nil and shardInfo.shardId ~= "?" then
    for _, s in ipairs(map.shards) do
      if s.id == shardInfo.shardId then
        currentShard = s
        break
      end
    end
  end

  if currentShard ~= nil then
    local lineColor = ImGui.GetColorU32(1.0, 0.3, 0.3, 1.0)
    local halfSize = RADAR_SIZE / 2
    for _, ring in ipairs(currentShard.boundaryRings) do
      local n = #ring
      for i = 1, n do
        local a = ring[i]
        local b = ring[(i % n) + 1]
        local ax, ay = (a[1] - pos.x) * scale, (a[2] - pos.y) * scale
        local bx, by = (b[1] - pos.x) * scale, (b[2] - pos.y) * scale
        -- Ne dessine que les arêtes qui touchent au moins partiellement le cadre du radar.
        if (math.abs(ax) <= halfSize or math.abs(bx) <= halfSize)
            and (math.abs(ay) <= halfSize or math.abs(by) <= halfSize) then
          drawList:AddLine(cx + ax, cy + ay, cx + bx, cy + by, lineColor, 2.0)
        end
      end
    end

    local bufferPx = (map.radius.base or 0) * scale
    if bufferPx > 0 then
      local bufferColor = ImGui.GetColorU32(1.0, 0.7, 0.0, 0.6)
      drawList:AddCircle(cx, cy, bufferPx, bufferColor, 32, 2.0)
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
    ImGui.Text("Shard: " .. (shardInfo.shardName or shardInfo.shardId))
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

-- Bindable dans l'overlay CET lui-même (touche ~ par défaut > onglet "Bindings"/"Hotkeys"), PAS
-- dans le menu Paramètres > Input du jeu — aucune touche imposée par défaut, l'utilisateur choisit
-- la sienne dans cet onglet. registerHotkey est l'API CET standard pour ça (persistée par CET).
registerHotkey('TesseraHUD_Toggle', 'Tessera HUD : afficher/masquer', function()
  TesseraHUD.visible = not TesseraHUD.visible
  print("[TesseraHUD] visibilité -> " .. tostring(TesseraHUD.visible))
end)

print("[TesseraHUD] hotkey enregistré avec succès — le mod a fini de charger sans erreur")
