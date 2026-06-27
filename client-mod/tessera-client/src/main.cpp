// Plugin RED4ext minimal « Tessera » — preuve de la chaîne de livraison client.
//
// Ce plugin ne fait RIEN de réseau : il se contente de se charger dans Cyberpunk 2077
// (via le loader RED4ext) et d'écrire une ligne de log au démarrage. Son seul rôle est de
// VALIDER de bout en bout : cloud-build Windows → emballage modset signé → install par le
// launcher → chargement réel en jeu. C'est l'équivalent côté client du `probe` côté serveur.
//
// Une fois cette chaîne verte, on y verse le vrai port Cyberverse (GNS + FlatBuffers).
//
// API ciblée : RED4ext SDK 1.0.0 (API v1, namespace RED4ext::v1). Ce SDK cible le jeu 2.31
// (RED4EXT_V1_RUNTIME_VERSION_LATEST == ..._2_31) ; on se déclare néanmoins RUNTIME_INDEPENDENT
// car ce plugin n'accroche aucune adresse précise de la build du jeu.

#include <RED4ext/RED4ext.hpp>

// Query — le loader lit notre identité avant de nous charger.
RED4EXT_C_EXPORT void RED4EXT_CALL Query(RED4ext::v1::PluginInfo* aInfo)
{
    aInfo->name = L"TesseraClient";
    aInfo->author = L"TesseraSynth";
    aInfo->version = RED4EXT_V1_SEMVER(0, 1, 0);
    aInfo->runtime = RED4EXT_V1_RUNTIME_INDEPENDENT; // loader-only, pas lié à une build du jeu
    aInfo->sdk = RED4EXT_V1_SDK_VERSION_CURRENT;
}

// Main — appelé au chargement et au déchargement du plugin par le loader.
RED4EXT_C_EXPORT bool RED4EXT_CALL Main(RED4ext::v1::PluginHandle aHandle, RED4ext::v1::EMainReason aReason,
                                        const RED4ext::v1::Sdk* aSdk)
{
    switch (aReason)
    {
    case RED4ext::v1::EMainReason::Load:
        aSdk->logger->Info(aHandle,
            "Tessera chargé — plugin client minimal v0.1.0. La chaîne de livraison fonctionne.");
        break;

    case RED4ext::v1::EMainReason::Unload:
        aSdk->logger->Info(aHandle, "Tessera déchargé.");
        break;
    }

    return true;
}

// Supports — version d'API que ce plugin attend du loader (API v1).
RED4EXT_C_EXPORT uint32_t RED4EXT_CALL Supports()
{
    return RED4EXT_API_VERSION_1;
}
