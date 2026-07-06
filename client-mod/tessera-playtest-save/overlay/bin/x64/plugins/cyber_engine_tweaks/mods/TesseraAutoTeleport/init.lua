-- Téléporte automatiquement le joueur près de la frontière de shard (x=2450) une fois la session
-- chargée, sans action manuelle en console. Complète TesseraAutoLoad (qui charge la save) : dès
-- que le joueur devient disponible (Game.GetPlayer() non nil, sondé chaque frame), téléporte UNE
-- SEULE fois via Game.GetTeleportationFacility():Teleport(...) — technique confirmée par le wiki
-- officiel CET (wiki.redmodding.org/cyber-engine-tweaks/teleportation-locations) et déjà testée
-- avec succès manuellement en jeu par Lucas (2026-07-06, depuis le monde ouvert).
--
-- Pourquoi ça ne déclenche PAS l'anti-triche serveur : le tout premier PositionUpdate envoyé
-- après la connexion n'est jamais soumis à la vérification de plausibilité côté serveur
-- (`last_pos_at` n'existe pas encore à ce stade — cf. tessera-core/server/src/gateway.rs). Peu
-- importe la distance entre la position embarquée dans la save et cette destination, le serveur
-- l'accepte tel quel : pas besoin de rang GameMaster ni de message protocole dédié pour ça.
--
-- PIN IN-GAME : jamais testé en conditions réelles DANS ce hook précis (seulement testé à la main
-- depuis la console CET, cf. commit précédent). À vérifier : le joueur doit être dans le monde
-- ouvert au moment du déclenchement (comme observé manuellement) — si le hook se déclenche trop
-- tôt (encore dans l'appartement/un intérieur), la téléportation peut échouer/annuler comme
-- constaté avec la commande console manuelle.

local TARGET_X = 2460.0
local TARGET_Y = 1270.0
local TARGET_Z = 130.0

local teleported = false

registerForEvent('onUpdate', function()
  if teleported then return end
  local player = Game.GetPlayer()
  if player == nil then return end

  teleported = true
  Game.GetTeleportationFacility():Teleport(
    player,
    ToVector4 { x = TARGET_X, y = TARGET_Y, z = TARGET_Z, w = 1 },
    ToEulerAngles { roll = 0, pitch = 0, yaw = 0 }
  )
end)
