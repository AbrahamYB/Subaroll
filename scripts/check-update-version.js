const fs = require('node:fs');

const published = JSON.parse(fs.readFileSync(process.argv[2], 'utf8')).version;
const next = JSON.parse(fs.readFileSync('src-tauri/tauri.conf.json', 'utf8')).version;
function parts(version) {
  if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error(`Invalid updater version: ${version}`);
  return version.split('.').map(Number);
}
const oldParts = parts(published);
const newParts = parts(next);
const difference = newParts.findIndex((part, i) => part !== oldParts[i]);
if (difference < 0 || newParts[difference] < oldParts[difference]) {
  throw new Error(`Build version ${next} must be newer than the published updater version ${published}.`);
}
console.log(`Updater version ${next} is newer than ${published}.`);
