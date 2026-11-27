// Entry for the cheerio bundle: the parse5-backed build without its network and stream loaders,
// which throw instead, since scripts have no network, filesystem or Node streams.
export * from "cheerio";
const unavailable = (name) => () => {
  throw new Error(`cheerio.${name} is not available here`);
};
export const fromURL = unavailable("fromURL");
export const loadBuffer = unavailable("loadBuffer");
export const stringStream = unavailable("stringStream");
export const decodeStream = unavailable("decodeStream");
