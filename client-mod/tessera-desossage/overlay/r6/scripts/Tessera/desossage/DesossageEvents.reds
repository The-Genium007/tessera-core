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

public func Tessera_ApplyQuestTriggers(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  if e.active {
    return;
  }
  // PIN IN-GAME : bloquer les déclencheurs/donneurs de quêtes + appels fixers (pas d'effacement de contenu).
  FTLog(s"[Tessera/Desossage] (stub) déclencheurs de quêtes → bloqués");
}

public func Tessera_ApplyTutorials(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  if e.active {
    return;
  }
  // PIN IN-GAME : désactiver les flags/pop-ups tutoriel.
  FTLog(s"[Tessera/Desossage] (stub) tutoriels → coupés");
}
