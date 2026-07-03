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

// Couvre le volet « appels fixers » de questTriggers (symbole réel, confirmé via le dump RTTI
// du jeu — GameInstance.GetPhoneManager -> questPhoneManager.ApplyPhoneCallRestriction(Bool)).
// Best-effort : bloque les appels entrants (donc les gigs/side-quests poussés par téléphone),
// mais PAS les déclencheurs de proximité ni les donneurs de quête in-world — ceux-là restent
// des PIN IN-GAME (aucun symbole vérifié trouvé côté déclencheurs de zone/PNJ).
//
// CONFIRMÉ EN JEU (2026-07-03) : effet observable immédiat sans attendre un vrai appel — le jeu
// verrouille l'icône de sélection de station radio tant que les appels sont autorisés (icône
// rouge). Décocher (e.active=false → ApplyPhoneCallRestriction(true)) déverrouille l'icône
// (rouge → bleu). C'est le moyen de vérif de référence pour ce levier en test manuel.
public func Tessera_ApplyQuestTriggers(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  GameInstance.GetPhoneManager(game).ApplyPhoneCallRestriction(!e.active);
  if e.active {
    FTLog(s"[Tessera/Desossage] déclencheurs de quêtes → appels fixers réactivés");
  } else {
    FTLog(s"[Tessera/Desossage] déclencheurs de quêtes → appels fixers bloqués (ApplyPhoneCallRestriction)");
  }
}

// Recherche (script décompilé officiel) : questTutorialManager n'expose que
// RequestToCloseOverlay(overlayId) — ferme un overlay déjà ouvert, aucun moyen confirmé
// d'empêcher l'ouverture en premier lieu. Confirmé insuffisant, pas juste non-trouvé.
public func Tessera_ApplyTutorials(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  if e.active {
    return;
  }
  FTLog(s"[Tessera/Desossage] (stub) tutoriels → coupés");
}
