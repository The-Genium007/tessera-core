module Tessera.Desossage

// ─────────────────────────────────────────────────────────────────────────────
// Pont HUD moniteur de cohérence de shard (spec 2026-07-18-hud-moniteur-coherence-shard.md,
// Phase C Task 6/7). Le HUD Lua (TesseraHUD/init.lua) appelle ces méthodes sur le joueur pour
// comparer son calcul local (shard-map.json) au placement AUTORITAIRE poussé par le serveur
// (ServerMsg::ShardAssignment), et afficher le nombre de joueurs visibles.
//
// La donnée réelle vit dans le netcode C++ du fork (NetworkGameSystem, The-Genium007/Cyberverse) :
// c'est lui qui décode ShardAssignment sur le fil et expose les trois `native func`
// Tessera_GetServerShard/Overlaps/VisiblePlayerCount (cf. NetworkGameSystem.reds/.h/.cpp du fork).
// Ces wrappers @addMethod(PlayerPuppet) ne font que déléguer, pour que le HUD puisse écrire
// `player:Tessera_GetServerShard()` — mirroir exact du pattern Tessera_SetLever (DesossageConsole.reds).
//
// GameInstance.GetNetworkGameSystem() est un global static natif déclaré côté fork ; il est visible
// ici car le modset et le RedscriptModule du fork compilent ensemble (même `r6/scripts`). Il peut
// renvoyer null (lancement hors serveur / netcode pas encore chargé) : chaque wrapper retombe alors
// sur une valeur neutre (chaîne vide / 0), que le HUD interprète comme « aucune donnée serveur »
// et n'affiche pas — jamais de crash du HUD sur un null.
// ─────────────────────────────────────────────────────────────────────────────

@addMethod(PlayerPuppet)
public func Tessera_GetServerShard() -> String {
  let net = GameInstance.GetNetworkGameSystem();
  if !IsDefined(net) {
    return "";
  }
  return net.Tessera_GetServerShard();
}

@addMethod(PlayerPuppet)
public func Tessera_GetServerOverlaps() -> String {
  let net = GameInstance.GetNetworkGameSystem();
  if !IsDefined(net) {
    return "";
  }
  return net.Tessera_GetServerOverlaps();
}

@addMethod(PlayerPuppet)
public func Tessera_GetVisiblePlayerCount() -> Int32 {
  let net = GameInstance.GetNetworkGameSystem();
  if !IsDefined(net) {
    return 0;
  }
  return net.Tessera_GetVisiblePlayerCount();
}
