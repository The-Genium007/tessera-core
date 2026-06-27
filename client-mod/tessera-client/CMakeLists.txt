# Plugin RED4ext minimal « Tessera » — compilé en .dll Windows (MSVC x64).
# Cible : Cyberpunk 2077 2.31 + RED4ext 1.30.0 (toolchain figée, ADR 0004).
#
# Build local (Windows, VS 2022) :
#   cmake -S client-mod/tessera-client -B build -A x64
#   cmake --build build --config Release
# Sortie : build/Release/TesseraClient.dll
#
# Le SDK RED4ext est récupéré par FetchContent (épinglé) — aucun sous-module à gérer.

cmake_minimum_required(VERSION 3.20)
project(TesseraClient LANGUAGES CXX VERSION 0.1.0)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

include(FetchContent)
FetchContent_Declare(
    RED4ext.SDK
    GIT_REPOSITORY https://github.com/WopsS/RED4ext.SDK.git
    GIT_TAG        v1.0.0   # API v1, contemporain du loader RED4ext 1.30.0 (jeu 2.31)
)
FetchContent_MakeAvailable(RED4ext.SDK)

add_library(${PROJECT_NAME} SHARED src/main.cpp)
target_link_libraries(${PROJECT_NAME} PRIVATE RED4ext.SDK)

# Le .dll doit s'appeler exactement TesseraClient.dll (pas de préfixe "lib").
set_target_properties(${PROJECT_NAME} PROPERTIES
    OUTPUT_NAME "TesseraClient"
    PREFIX ""
)
