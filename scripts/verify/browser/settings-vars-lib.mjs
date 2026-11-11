// The settings check's vars step. Imported by scripts/verify/browser/browser-check.mjs.
//
// On the actions form, a variable is added in the name and value rows and saved. The file keeps
// the key the earlier step edited and holds the variable as an inline table, and the form reads
// the saved value back.

import { readFileSync } from 'node:fs';
import { join } from 'node:path';

/**
 * @param {any} p the page
 * @param {any} srv the served repository
 * @param {string} cfg the config folder
 * @param {string} name the browser's name
 * @param {any} r the reporter
 */
export async function varsStep(p, srv, cfg, name, r) {
  const rows = '#f-action\\.0\\.vars ~ .kvrows';
  const saveOn = () => !(/** @type {HTMLButtonElement} */ (document.getElementById('setsave')))
    .disabled;
  await p.goto(srv.origin + '/_settings?f=actions', { waitUntil: 'load' });
  await p.click('#f-action\\.0\\.vars-add');
  await p.fill(`${rows} .kvname`, 'vault');
  await p.fill(`${rows} .kvval`, 'Notes "x"');
  await p.waitForFunction(saveOn, null, { timeout: 3000 });
  await Promise.all([p.waitForNavigation(), p.click('#setsave')]);
  const file = readFileSync(join(cfg, 'actions.toml'), 'utf8');
  const shown = await p.$eval(`${rows} .kvval`, (x) => /** @type {HTMLInputElement} */ (x).value);
  const want = 'vars = { vault = \'Notes "x"\' }';
  if (file.includes(want) && file.includes('label = "Mark it"') && shown === 'Notes "x"') {
    r.ok(`settings ${name}: a variable added in the form saves`);
  } else r.fail(`settings ${name}: vars save gave ${JSON.stringify(file)}, form shows ${shown}`);
}
