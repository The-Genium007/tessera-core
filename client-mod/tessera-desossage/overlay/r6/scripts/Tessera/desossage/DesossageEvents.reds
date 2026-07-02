module Tessera.Desossage

// Leviers événements & progression : rencontres ambiantes (par type), quêtes, tutoriels.
// STUB : journalise l'intention. Corps réel (spawn clusters, déclencheurs de quêtes) à pincer en jeu.

// Coupe (ou règle) une catégorie de rencontres ambiantes. `kind` = type (granularité).
public func Tessera_ApplyEncounterCategory(game: GameInstance, kind: CName, e: ref<DesossageEntry>) -> Void {
  let factor: Float = 0.0;
  if e.active { factor = e.density; }
  // PIN IN-GAME : régler la densité des spawn clusters de la catégorie `kind` à `factor`.
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

public func Tessera_ApplyTutorials(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  if e.active {
    return;
  }
  // PIN IN-GAME : désactiver les flags/pop-ups tutoriel.
  FTLog(s"[Tessera/Desossage] (stub) tutoriels → coupés");
}
