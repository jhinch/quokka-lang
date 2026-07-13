# Lexical Structure

Lexical analysis ("lexing") is the process of converting a stream of characters in a [source file](source-files.md)
into stream of [tokens](tokens.md).

[whitespace](whitespace.md) and [comments](comments.md) serve only to separate [tokens](tokens.md) within the input
stream and have no semantic significance.

## Source encoding

Before lexing can occur, a source file is decoded as a sequence of [Unicode](https://www.unicode.org/versions/latest/)
characters encoded in [UTF-8](https://www.unicode.org/versions/latest/core-spec/chapter-3/#G31703).