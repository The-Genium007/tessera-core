module Tessera.Desossage

// Leviers population : piétons, trafic véhicules, transit (métro/passants).
// STUB : journalise l'intention. Corps réel (densité des communautés / trafic) à pincer en jeu.

// Réel : CommunitySystem.ChangeDensityModifier(Float) / ResetDensityModifier() — confirmé par le
// script décompilé du jeu (CDPR-Modding-Documentation/Cyberpunk-Scripts, scripts/core/systems/
// communitySystem.script). Multiplicateur global de densité des foules ambiantes — 0.0 = vide,
// 1.0 = normal. (Note : EnableDynamicCrowdNullArea existe aussi sur cette classe pour des zones
// précises, mais ChangeDensityModifier est le levier global recherché ici, et beaucoup plus
// simple/sûr — pas de Box/WorldTransform à construire.)
public func Tessera_ApplyPedestrians(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  let factor: Float = 0.0;
  if e.active { factor = e.density; }
  GameInstance.GetCommunitySystem(game).ChangeDensityModifier(factor);
  FTLog(s"[Tessera/Desossage] piétons → densité \(factor) (ChangeDensityModifier)");
}

// Repeuplement observé en jeu (bug rapporté 2026-07-06) : la densité coupée revenait après
// quelques minutes, surtout en changeant de quartier (ChangeDensityModifier ne modifie que les
// communautés déjà chargées, les nouveaux secteurs streamés reçoivent les valeurs par défaut du
// moteur — confirmé via adamsmasher/cyberpunk et le mod Nexus "No Crowds and Cars" #248).
//
// RÉSOLU AUTREMENT : `engine/config/platform/pc/user.ini` `[Crowd]` (voir ce fichier dans
// l'overlay) coupe la densité au niveau moteur, relu à CHAQUE secteur streamé — confirmé en jeu
// par Lucas (2026-07-06, v2.31). C'est la solution retenue, pas un hook réactif.
//
// TENTATIVE ABANDONNÉE (2026-07-06) : un hook réactif @wrapMethod(PreventionSystem)
// OnDistrictAreaEntered(evt: ref<gamemappinsDistrictEnteredEvent>) avait été ajouté en filet de
// sécurité, mais a fait planter TOUTE la compilation redscript en jeu (confirmé via
// redscript_rCURRENT.log : "[UNRESOLVED_METHOD] this signature does not match any existing
// method") — la visibilité `protected` choisie par analogie n'était pas la bonne, et le PIN
// IN-GAME laissé dans le code annonçait justement ce risque. Retiré plutôt que deviné une
// deuxième fois à l'aveugle : le fix ini suffit, pas besoin de reprendre cette piste sauf si un
// vrai pop-in visible est un jour constaté malgré l'ini.

// Recherche confirmée via dump RTTI complet (WopsS/RED4ext.NativeDB) : GameInstance.GetTrafficSystem
// retourne `worldTrafficScriptInterface`, qui n'expose qu'UNE seule méthode dans tout le jeu :
// IsPathIntersectingWithTraffic (requête, pas de setter de densité). Parent = IScriptable (rien
// d'hérité d'utile). AITrafficMovementSystem (la classe interne derrière l'interface scriptée) est
// tout aussi vide — zéro méthode/champ propre à aucun niveau de sa chaîne de parenté. Piste
// "système de trafic dédié" définitivement épuisée côté RTTI.
//
// HYPOTHÈSE RENFORCÉE (2026-07-03, re-creusée) : `worldPopulationSpawnerNode` (le nœud de spawn
// placé dans le monde par le level design) a un champ `.isVehicle: Bool` — piétons ET véhicules
// sont donc le MÊME type de nœud de spawn, juste distingués par ce booléen, pas deux systèmes
// séparés. Les classes `populationModifier`/`populationSpawnModifier`/
// `populationPopulationSpawnParameter` qui gravitent autour sont de purs conteneurs de données
// (zéro méthode), cohérent avec un système consommé en interne par CommunitySystem plutôt qu'une
// API scriptable parallèle. Ça renforce nettement l'hypothèse que `ChangeDensityModifier` pilote
// aussi le trafic, sans qu'on ait trouvé de setter dédié séparé pour les véhicules.
//
// CONFIRMÉ EN JEU (2026-07-05, cf. tools/nativedb/findings.md) : comparaison avant/après au même
// endroit — `pedestrians` décoché = 0 véhicule sur la rue ; `pedestrians` coché (rien d'autre
// changé) = plusieurs véhicules apparaissent. `ChangeDensityModifier` pilote donc bien les deux.
// Ce stub est un DOUBLON, pas un système à implémenter — candidat à retirer (lever + case UI dans
// TesseraDesossage/init.lua) dans un prochain nettoyage, laissé en l'état pour ne pas changer le
// schéma DesossageConfig pendant la session de test.
public func Tessera_ApplyTraffic(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  let factor: Float = 0.0;
  if e.active { factor = e.density; }
  // PIN IN-GAME : régler la densité du trafic véhicules à `factor`.
  FTLog(s"[Tessera/Desossage] (stub) trafic → densité \(factor)");
}

// Recherche (dump RTTI complet) : seule classe pertinente trouvée pour "métro/NCART" est
// `NcartTimetableControllerPS` (PS de device, même famille que VendingMachineControllerPS) — mais
// ses méthodes propres (GetCurrentTimeToDepart, ResetTimeToDepart, UpdateCurrentTimeToDepart...)
// pilotent l'affichage du panneau d'horaires en station, pas le spawn/densité de passagers ou de
// PNJ de transit. Piste écartée pour ce levier précis. Aucune autre classe "Metro"/"Transit"
// pertinente dans les 14 094 classes RTTI. PIN IN-GAME : reste à chercher côté script décompilé
// (peut-être un tag de spawn communautaire dédié plutôt qu'un système à part, comme pour
// ncpdHustles/randomEncounters — cf. DesossageEvents.reds).
public func Tessera_ApplyTransit(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  let factor: Float = 0.0;
  if e.active { factor = e.density; }
  // PIN IN-GAME : régler la densité du transit (métro NCART / passants de transit) à `factor`.
  FTLog(s"[Tessera/Desossage] (stub) transit → densité \(factor)");
}
