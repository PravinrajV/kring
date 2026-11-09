// Some NEAR wallet libraries expect Node's Buffer in the browser.
import { Buffer } from 'buffer';
globalThis.Buffer ??= Buffer;
