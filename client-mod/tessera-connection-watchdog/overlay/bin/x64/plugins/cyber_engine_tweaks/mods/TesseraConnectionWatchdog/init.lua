-- Surveille l'état de connexion réseau exposé par le plugin natif (NetworkGameSystem.FullyConnected,
-- cf. client/red4ext/src/NetworkGameSystem.cpp dans le fork Cyberverse, ConnectionStatusChangedCallback)
-- et affiche une alerte persistante à l'écran dès que la connexion tombe (crash serveur, coupure
-- réseau...).
--
-- Pourquoi ce mod : FullyConnected bascule à false dans le callback GNS côté C++ mais ce n'est
-- qu'un booléen interne, consulté uniquement par Cyberverse.reds (OnSavesForLoadReady, pour savoir
-- s'il faut différer un chargement) — rien ne déconnecte le joueur ni ne l'avertit. Signalé en
-- playtest (2026-07-07) : "quand le serveur crash ou se coupe, on n'est pas déco du jeu". Plutôt
-- que de modifier le plugin natif (rebuild + nouvelle release netcode-v*, cycle long), on lit
-- FullyConnected depuis CET : c'est une propriété RTTI déjà exposée et utilisée telle quelle côté
-- redscript (`networkSystem.FullyConnected`, Cyberverse.reds ligne ~20) — accès en lecture
-- identique attendu côté Lua CET (même pont RTTI que Game.GetPlayer()/Game.GetTeleportationFacility(),
-- déjà confirmés fonctionnels dans ce projet). GetNetworkGameSystem() est un @addMethod(GameInstance)
-- du fork (pas natif CDPR), mais exposé par RTTI comme n'importe quelle autre méthode GameInstance.
--
-- PIN IN-GAME : jamais testé. À confirmer au premier lancement : (1) Game.GetNetworkGameSystem()
-- résout bien cette méthode custom du fork depuis CET, (2) la propriété .FullyConnected se lit
-- comme un simple champ Lua sur l'objet retourné. Le print() de diagnostic ci-dessous couvre les
-- deux cas d'échec.

print("[TesseraConnectionWatchdog] fichier init.lua en cours de chargement...")

local wasConnected = nil -- nil = jamais observé ; true/false = dernier état connu
local disconnectedSince = nil

local function checkConnection()
  local ok, result = pcall(function()
    local net = Game.GetNetworkGameSystem()
    if net == nil then return nil end
    return net.FullyConnected
  end)

  if not ok then
    print("[TesseraConnectionWatchdog] ERREUR pendant la lecture de FullyConnected : " .. tostring(result))
    return
  end

  local connected = result
  if connected == nil then
    return -- système pas encore prêt (avant la 1re connexion), on réessaiera la frame suivante
  end

  if wasConnected == true and connected == false then
    disconnectedSince = os.clock()
    print("[TesseraConnectionWatchdog] connexion perdue (FullyConnected: true -> false)")
  elseif wasConnected == false and connected == true then
    disconnectedSince = nil
    print("[TesseraConnectionWatchdog] connexion rétablie (FullyConnected: false -> true)")
  elseif wasConnected == nil then
    print("[TesseraConnectionWatchdog] état initial FullyConnected = " .. tostring(connected))
  end

  wasConnected = connected
end

registerForEvent('onUpdate', function()
  checkConnection()
end)

registerForEvent('onDraw', function()
  if disconnectedSince == nil then return end

  ImGui.SetNextWindowPos(20, 20, ImGuiCond.Always)
  ImGui.SetNextWindowBgAlpha(0.85)
  ImGui.Begin("Connexion serveur", true, ImGuiWindowFlags.AlwaysAutoResize)
  ImGui.TextColored(1.0, 0.2, 0.2, 1.0, "CONNEXION AU SERVEUR PERDUE")
  ImGui.Text(string.format(
    "Depuis %.0fs - le jeu continue en local, rien ne sera synchronise avec le serveur.",
    os.clock() - disconnectedSince
  ))
  ImGui.End()
end)

print("[TesseraConnectionWatchdog] init.lua chargé jusqu'au bout — surveillance active")
