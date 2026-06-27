// Plugin RED4ext minimal « Tessera » — preuve de la chaîne de livraison client.
//
// Ce plugin ne fait RIEN de réseau : il se contente de se charger dans Cyberpunk 2077
// (via le loader RED4ext) et d'écrire une ligne de log au démarrage. Son seul rôle est de
// VALIDER de bout en bout : cloud-build Windows → emballage modset signé → install par le
// launcher → chargement réel en jeu. C'est l'équivalent côté client du `probe` côté serveur.
//
// Une fois cette chaîne verte, on y verse le vrai port Cyberverse (GNS + FlatBuffers).
//
// On n'utilise QUE les macros du SDK (RED4EXT_SDK_LATEST / RED4EXT_API_VERSION_LATEST) :
// le plugin colle ainsi automatiquement à la version de SDK contre laquelle il est compilé.

#include <RED4ext/RED4ext.hpp>

// Query — le loader lit notre identité avant de nous charger.
RED4EXT_C_EXPORT void RED4EXT_CALL Query(RED4ext::PluginInfo* aInfo)
{
    aInfo->name = L"TesseraClient";
    aInfo->author = L"TesseraSynth";
    aInfo->version = RED4EXT_SEMVER(0, 1, 0);
    aInfo->runtime = RED4EXT_RUNTIME_INDEPENDENT; // pas lié à une build précise du jeu
    aInfo->sdk = RED4EXT_SDK_LATEST;
}

// Main — appelé au chargement et au déchargement du plugin par le loader.
RED4EXT_C_EXPORT bool RED4EXT_CALL Main(RED4ext::PluginHandle aHandle, RED4ext::EMainReason aReason,
                                        const RED4ext::Sdk* aSdk)
{
    switch (aReason)
    {
    case RED4ext::EMainReason::Load:
        aSdk->logger->Info(aHandle,
            "Tessera chargé — plugin client minimal v0.1.0. La chaîne de livraison fonctionne.");
        break;

    case RED4ext::EMainReason::Unload:
        aSdk->logger->Info(aHandle, "Tessera déchargé.");
        break;
    }

    return true;
}

// Supports — version d'API que ce plugin attend du loader.
RED4EXT_C_EXPORT uint32_t RED4EXT_CALL Supports()
{
    return RED4EXT_API_VERSION_LATEST;
}
