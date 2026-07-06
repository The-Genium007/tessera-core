-- Lancement direct playtest : charge automatiquement UNE sauvegarde précise (identifiée par nom,
-- pas "la dernière") dès que le menu principal a la liste des saves, pour arriver dans le jeu sans
-- toucher au menu ni voir Continuer/Nouvelle partie. Combiné à -skipStartScreen (déjà géré côté
-- launcher/launch args), ça couvre toute la chaîne "clic Play → dans le monde".
--
-- Technique confirmée par deux sources réelles indépendantes (2026-07-06) :
--   - github.com/Nats-ji/CP77-Skip-Main-Menu (CET Lua) : ObserveAfter sur
--     SingleplayerMenuGameController.OnSavesForLoadReady, puis charge une save via le handler.
--   - github.com/psiberx/cp2077-playground/blob/main/DevTools/AutoContinue.reds (redscript,
--     @wrapMethod) : même hook, même événement.
-- `LoadSaveInGame(Int32)` confirmé par le dump RTTI local (tools/nativedb/search.py show
-- SingleplayerMenuGameController --deep) — héritée de gameuiSaveHandlingController, prend un INDEX
-- (pas un nom), d'où le besoin de chercher l'index dans le tableau `saves` reçu par
-- OnSavesForLoadReady(array:String) plutôt que d'utiliser LoadLastCheckpoint (qui ignore le nom et
-- charge "la dernière utilisée", pas fiable pour cibler CETTE save précise).
--
-- PIN IN-GAME : jamais testé. Deux points à vérifier en jeu :
--   1. CET forward bien les arguments de la méthode observée au callback Lua (comportement standard
--      documenté de ObserveAfter, mais jamais vérifié sur CE hook précis) ;
--   2. le nom exact tel qu'il apparaît dans le tableau `saves` (nom de dossier de save, probablement
--      "ManualSave-20" ou une variante) — à ajuster ci-dessous si le match échoue silencieusement.

-- Nom (ou sous-chaîne) du dossier de sauvegarde à charger automatiquement. Laisser vide ("") pour
-- désactiver l'auto-load sans supprimer le mod (fallback : menu normal, aucun risque).
-- "TesseraPlaytest" : nom de dossier volontairement distinctif (pas "ManualSave-N") pour ne
-- jamais entrer en collision avec les sauvegardes propres du joueur — cf. save/TesseraPlaytest/
-- (save perso de Lucas, v2.31, StreetKid, fin Acte 2, non moddée).
local TARGET_SAVE_NAME = "TesseraPlaytest"

local alreadyLoaded = false

local function tryAutoLoad(controller, saves)
  if TARGET_SAVE_NAME == "" or alreadyLoaded then return end
  if saves == nil then return end

  for i, name in ipairs(saves) do
    if string.find(name, TARGET_SAVE_NAME, 1, true) ~= nil then
      alreadyLoaded = true
      controller:LoadSaveInGame(i - 1) -- tableau Lua 1-based -> index API 0-based
      return
    end
  end
end

registerForEvent('onInit', function()
  ObserveAfter('SingleplayerMenuGameController', 'OnSavesForLoadReady', function(controller, saves)
    tryAutoLoad(controller, saves)
  end)
end)
