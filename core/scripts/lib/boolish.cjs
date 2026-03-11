const TRUE_VALUES = new Set(["1", "true", "yes", "on"]);
const FALSE_VALUES = new Set(["0", "false", "no", "off"]);

const normalizeBoolish = (value) => String(value ?? "").trim().toLowerCase();

const parseBoolish = (value) => {
  const normalized = normalizeBoolish(value);
  if (!normalized) return undefined;
  if (TRUE_VALUES.has(normalized)) return true;
  if (FALSE_VALUES.has(normalized)) return false;
  return undefined;
};

const resolveBoolishFlag = (value, defaultValue = false, label = "flag") => {
  const normalized = normalizeBoolish(value);
  if (!normalized) return defaultValue;
  const parsed = parseBoolish(normalized);
  if (parsed === undefined) {
    throw new Error(`Invalid ${label}: expected one of 1/true/yes/on or 0/false/no/off`);
  }
  return parsed;
};

const stringMapFlag = (values, key) => parseBoolish(values?.[key]) === true;

module.exports = {
  normalizeBoolish,
  parseBoolish,
  resolveBoolishFlag,
  stringMapFlag,
};
