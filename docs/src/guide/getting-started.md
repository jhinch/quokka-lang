# Getting started

This part of the book takes you from nothing to a Game Boy ROM, compiled by Quokka, that shows a picture you drew
yourself. Work through the chapters in order:

1. **[Installation](installation.md)**: install RGBDS, which Quokka uses to link ROMs, and then install the `quokka`
   compiler, either from a pre-built release or by building it from source.
2. **[Hello, Quokka!](hello-quokka.md)**: a tutorial that turns a 160×144 pixel PNG into Game Boy tile data and
   writes a Quokka program to display it. Along the way it covers how the Game Boy draws its background, and the
   Quokka globals, constants, arrays and loops used to drive it.

Once you have finished the tutorial, go to the [reference](../reference/lexical-structure.md). It gives the full
details of every part of the language, such as the [types](../reference/types.md),
[attributes](../reference/attributes.md) and [expressions](../reference/expressions.md) that the tutorial only
touches on. The reference doesn't need to be read in order, so jump to whatever you need.

You don't need to know Game Boy assembly to follow along, but Quokka works close to the hardware. The
[Pan Docs](https://gbdev.io/pandocs/) and the [GB ASM tutorial](https://gbdev.io/gb-asm-tutorial/) are both good
companions for learning how the Game Boy itself works.
