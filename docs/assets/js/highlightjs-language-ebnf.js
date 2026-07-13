/*!
  Highlight.js v11.11.1 (git: 08cb242e7d)
  (c) 2006-2025 Josh Goebel <hello@joshgoebel.com> and other contributors
  License: BSD-3-Clause
 */
/*!
BSD 3-Clause License

Copyright (c) 2006, Ivan Sagalaev.
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

* Redistributions of source code must retain the above copyright notice, this
  list of conditions and the following disclaimer.

* Redistributions in binary form must reproduce the above copyright notice,
  this list of conditions and the following disclaimer in the documentation
  and/or other materials provided with the distribution.

* Neither the name of the copyright holder nor the names of its
  contributors may be used to endorse or promote products derived from
  this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
*/
/*! `ebnf` grammar compiled for Highlight.js 11.11.1 */
  (function(){
    var hljsGrammar = (function () {
  'use strict';

  /*
  Language: Extended Backus-Naur Form
  Author: Alex McKibben <alex@nullscope.net>
  Website: https://en.wikipedia.org/wiki/Extended_Backus–Naur_form
  Category: syntax
  */

  /** @type LanguageFn */
  function ebnf(hljs) {
    const commentMode = hljs.COMMENT(/\(\*/, /\*\)/);

    const nonTerminalMode = {
      className: "attribute",
      begin: /^[ ]*[a-zA-Z]+([\s_-]+[a-zA-Z]+)*/
    };

    const specialSequenceMode = {
      className: "meta",
      begin: /\?.*\?/
    };

    const ruleBodyMode = {
      begin: /=/,
      end: /[.;]/,
      contains: [
        commentMode,
        specialSequenceMode,
        {
          // terminals
          className: 'string',
          variants: [
            hljs.APOS_STRING_MODE,
            hljs.QUOTE_STRING_MODE,
            {
              begin: '`',
              end: '`'
            }
          ]
        }
      ]
    };

    return {
      name: 'Extended Backus-Naur Form',
      illegal: /\S/,
      contains: [
        commentMode,
        nonTerminalMode,
        ruleBodyMode
      ]
    };
  }

  return ebnf;

})();

    hljs.registerLanguage('ebnf', hljsGrammar);
  })();
