cmake_minimum_required(VERSION 3.16)
project(sharkvis C)

set(CMAKE_C_STANDARD 17)
set(CMAKE_C_STANDARD_REQUIRED ON)

add_compile_definitions(_POSIX_C_SOURCE=200809L _DEFAULT_SOURCE)
add_compile_options(-Wall -Wextra -Wno-unused-parameter)

find_package(Threads REQUIRED)
find_package(Python3 COMPONENTS Interpreter REQUIRED)

set(UNIFONT_HEX ${CMAKE_SOURCE_DIR}/third_party/unifont.hex)
set(UNIFONT_GEN_C ${CMAKE_BINARY_DIR}/unifont_data.c)
set(UNIFONT_GEN_H ${CMAKE_BINARY_DIR}/unifont_data.h)

add_custom_command(
    OUTPUT ${UNIFONT_GEN_C} ${UNIFONT_GEN_H}
    COMMAND ${Python3_EXECUTABLE} ${CMAKE_SOURCE_DIR}/scripts/unifont_hex_to_c.py
            ${UNIFONT_HEX} ${UNIFONT_GEN_C} ${UNIFONT_GEN_H}
    DEPENDS ${CMAKE_SOURCE_DIR}/scripts/unifont_hex_to_c.py ${UNIFONT_HEX}
    COMMENT "Generating unifont glyph table"
)

add_executable(sharkvis
    src/main.c
    src/common.c
    src/term.c
    src/config.c
    src/fft.c
    src/dsp.c
    src/pulse.c
    src/audio.c
    src/raw.c
    src/state.c
    src/render.c
    src/settings.c
    src/lyrics.c
    src/mpris.c
    src/musixmatch.c
    src/unifont.c
    ${UNIFONT_GEN_C}
)

target_include_directories(sharkvis PRIVATE src ${CMAKE_BINARY_DIR})
target_link_libraries(sharkvis PRIVATE m Threads::Threads)

install(TARGETS sharkvis DESTINATION bin)
