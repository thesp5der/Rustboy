#!/bin/sh
# Assemble pong.asm into a Game Boy ROM (needs RGBDS: https://rgbds.gbdev.io)
#   ./build.sh            -> pong.gb
#   ./build.sh autoplay   -> pong-autoplay.gb (the player paddle plays itself; for testing)
set -e
cd "$(dirname "$0")"
if [ "$1" = autoplay ]; then OUT=pong-autoplay; DEFS="-D AUTOPLAY"; else OUT=pong; DEFS=""; fi
rgbasm -Weverything $DEFS -o $OUT.o pong.asm
rgblink -n $OUT.sym -o $OUT.gb $OUT.o
rgbfix -v -p 0xFF -t "PONG" $OUT.gb
rm -f $OUT.o
echo "built $(pwd)/$OUT.gb"
