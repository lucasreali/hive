import { GlobalRegistrator } from "@happy-dom/global-registrator";

GlobalRegistrator.register();

// happy-dom has no FontFaceSet: every font is already loaded.
Object.defineProperty(document, "fonts", { value: { load: async () => [] } });
