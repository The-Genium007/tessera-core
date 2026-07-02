module Tessera.Desossage

// Leviers événements & progression : rencontres ambiantes (par type), quêtes, tutoriels.
// STUB : journalise l'intention. Corps réel (spawn clusters, déclencheurs de quêtes) à pincer en jeu.

// Coupe (ou règle) une catégorie de rencontres ambiantes. `kind` = type (granularité).
// Recherche (script décompilé officiel, CDPR-Modding-Documentation/Cyberpunk-Scripts) : aucune
// classe/système « Hustle »/« ScannerHustle »/« CrimeSpawn » n'existe dans le jeu — confirmé
// absent, pas juste non-trouvé (recherché sur l'intégralité du dépôt de scripts décompilés).
// Les hustles NCPD sont probablement des entrées de spawn communautaire taguées + de la donnée
// TweakDB/quête, pas un système dédié désactivable. cyberpsychos : un système existe bien
// (`CyberpsychoEncountersSystem`, vu via GameInstance.GetCyberpsychoEncountersSystem dans un mod
// publié) mais CE mod l'AJOUTE/le remplace — pas de preuve que c'est le système natif vanilla, à
// vérifier avant d'utiliser ce nom. randomEncounters : pas encore cherché spécifiquement.
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
