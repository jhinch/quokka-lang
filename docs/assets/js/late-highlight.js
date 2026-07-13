// Unfortunately, mdbook doesn't allow for registering highlight.js language definitions as custom JS
// as the book.js which performs the highlighting happens before the custom scripts.
//
// Rerun the highlighting for the languages registered as custom scripts
document.querySelectorAll('code:is(.language-ebnf, .language-quokka):not(.header code)')
        .forEach(block => hljs.highlightBlock(block));
