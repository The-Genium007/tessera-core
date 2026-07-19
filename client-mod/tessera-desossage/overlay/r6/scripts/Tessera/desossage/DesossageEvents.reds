module Tessera.Desossage

// Leviers événements & progression : rencontres ambiantes (par type), quêtes, tutoriels.
// STUB : journalise l'intention. Corps réel (spawn clusters, déclencheurs de quêtes) à pincer en jeu.

// Coupe (ou règle) une catégorie de rencontres ambiantes. `kind` = type (granularité).
// Recherche (script décompilé officiel, CDPR-Modding-Documentation/Cyberpunk-Scripts) : aucune
// classe/système « Hustle »/« ScannerHustle »/« CrimeSpawn » n'existe dans le jeu — confirmé
// absent, pas juste non-trouvé (recherché sur l'intégralité du dépôt de scripts décompilés).
//
// DURCI par dump RTTI complet (2026-07-03, WopsS/RED4ext.NativeDB, 14 094 classes + les 98
// accesseurs GameInstance.GetXXXSystem natifs) : ZÉRO classe contenant "psycho", "encounter" ou
// "hustle" dans tout le RTTI du jeu, et aucun `GetCyberpsychoEncountersSystem`/`GetEncounterSystem`/
// `GetHustleSystem` parmi les 98 systèmes natifs. Confirme que `CyberpsychoEncountersSystem`
// (vu dans un mod publié) est bien ajouté par CE mod, pas un système vanilla — ne PAS l'utiliser
// comme s'il était natif. Les 3 leviers (ncpdHustles, randomEncounters, cyberpsychos) sont
// structurellement absents du RTTI : soit ce sont des entrées de spawn communautaire taguées +
// donnée TweakDB/quête (pas un système dédié désactivable), soit il faudrait un hook C++ natif
// (dernier recours, cf. mémoire projet triage RTTI → script décompilé → hook C++). Prochaine étape
// si on veut aller plus loin : chercher côté script décompilé un tag/CName de spawn communautaire
// dédié (`communitySpawnEntry`/`communitySquadInitializer`, vus dans le RTTI) plutôt qu'un système.
public func Tessera_ApplyEncounterCategory(game: GameInstance, kind: CName, e: ref<DesossageEntry>) -> Void {
  let factor: Float = 0.0;
  if e.active { factor = e.density; }
  FTLog(s"[Tessera/Desossage] (stub) rencontres \(kind) → densité \(factor)");
}

// Levier phoneCalls (ex-questTriggers, renommé 2026-07-19) — coupe les appels téléphoniques
// entrants « définitivement » (demande de Lucas : plus d'appels sur la map vide). Symbole réel,
// confirmé via le dump RTTI du jeu — GameInstance.GetPhoneManager ->
// questPhoneManager.ApplyPhoneCallRestriction(Bool) : bloque tous les appels entrants (fixers,
// gigs/side-quests poussés par téléphone).
//
// Ce que ce levier NE couvre PAS (et pourquoi ce n'est plus un trou) : les déclencheurs de
// proximité / donneurs de quête in-world (qui n'ont jamais transité par le téléphone). Ils sont
// désormais pris en charge côté natif par le blocage des spawn nodes (tessera-client phase 2,
// `QuestPhaseInstance::ExecuteNode` → questSpawnManagerNodeDefinition bloqué) — le donneur ne
// spawn plus, donc ne déclenche plus. Les deux leviers se complètent ; aucun symbole redscript
// vérifié n'existe côté déclencheurs de zone/PNJ, c'est bien le C++ qui ferme ce trou.
//
// CONFIRMÉ EN JEU (2026-07-03) : effet observable immédiat sans attendre un vrai appel — le jeu
// verrouille l'icône de sélection de station radio tant que les appels sont autorisés (icône
// rouge). Décocher (e.active=false → ApplyPhoneCallRestriction(true)) déverrouille l'icône
// (rouge → bleu). C'est le moyen de vérif de référence pour ce levier en test manuel.
public func Tessera_ApplyPhoneCalls(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  GameInstance.GetPhoneManager(game).ApplyPhoneCallRestriction(!e.active);
  if e.active {
    FTLog(s"[Tessera/Desossage] appels téléphoniques → réactivés");
  } else {
    FTLog(s"[Tessera/Desossage] appels téléphoniques → bloqués (ApplyPhoneCallRestriction)");
  }
}

// CORRIGÉ (2026-07-07) : l'ancienne piste (questTutorialManager.RequestToCloseOverlay) ne
// faisait que fermer un overlay déjà ouvert, jamais empêcher son ouverture — confirmé
// insuffisant. Remplacée par le fact save `disable_tutorials`, retrouvé dans un vrai export
// CyberCAT (2026-07-06) : non préfixé par un code de quête (donc pas de la progression
// scénaristique, un simple interrupteur d'état — même famille que `radio_on`/`tv_on` cités en
// exemple par le wiki officiel RedModding). `QuestsSystem.SetFactStr` est l'API native utilisée
// pour poser des facts au runtime — confirmée en lisant le code réel du mod publié
// "Immersive Gigs" (Nexus #24604, `init.lua`) : `Game.GetQuestsSystem():SetFactStr(name, value)`.
// PIN IN-GAME : jamais testé — à confirmer (plus de popup de tutoriel en jeu).
public func Tessera_ApplyTutorials(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  // e.active = système vanilla ACTIF (tutoriels affichés) ; défaut false = monde vide =
  // disable_tutorials=1. Ne pas inverser : cf. le même piège corrigé sur ce fichier ci-dessous.
  let value: Int32 = 1;
  if e.active { value = 0; }
  GameInstance.GetQuestsSystem(game).SetFact(n"disable_tutorials", value);
  FTLog(s"[Tessera/Desossage] tutoriels → fact disable_tutorials=\(value) (SetFact)");
}

// Trafic aérien (AVs dans le ciel) : même mécanisme que les tutoriels ci-dessus — fact save
// `air_traffic_off` retrouvé dans le même export CyberCAT (2026-07-06), non préfixé par un code
// de quête. Posé au chargement via SetFactStr, pas besoin d'éditer la save partagée à la main.
// PIN IN-GAME : jamais testé — à confirmer (plus d'AVs visibles dans le ciel).
public func Tessera_ApplyAirTraffic(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  // Même polarité que tutorials ci-dessus : e.active=false (défaut, monde vide) → air_traffic_off=1.
  let value: Int32 = 1;
  if e.active { value = 0; }
  GameInstance.GetQuestsSystem(game).SetFact(n"air_traffic_off", value);
  FTLog(s"[Tessera/Desossage] trafic aérien → fact air_traffic_off=\(value) (SetFact)");
}
