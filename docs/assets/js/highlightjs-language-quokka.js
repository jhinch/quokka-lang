(function () {
    function quokkaGrammar(hljs) {
        return {
            name: 'Quokka',
            keywords: 'as break const continue else fn global if let loop return struct then yield',
            contains: [
                hljs.C_LINE_COMMENT_MODE,
                {
                    className: 'number',
                    variants: [
                        {begin: '\\b0b([01_]+)'},
                        {begin: '\\b0o([0-7_]+)'},
                        {begin: '\\b0x([A-Fa-f0-9_]+)'},
                        {begin: '\\b(\\d[\\d_]*)'}
                    ],
                    relevance: 0
                }
            ]
        };
    }

    hljs.registerLanguage('quokka', quokkaGrammar);
})();