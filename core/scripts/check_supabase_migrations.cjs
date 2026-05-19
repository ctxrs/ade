#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");

function defaultMigrationsDir() {
  return path.join(__dirname, "..", "..", "supabase", "migrations");
}

function checkMigrationFiles(migrationsDir) {
  if (!fs.existsSync(migrationsDir)) {
    throw new Error(`missing Supabase migrations directory: ${migrationsDir}`);
  }

  const files = fs
    .readdirSync(migrationsDir)
    .filter((file) => file.endsWith(".sql"))
    .sort();

  if (files.length === 0) {
    throw new Error("no Supabase migrations found");
  }

  const seenFilenames = new Set();
  const versionToFile = new Map();

  for (const file of files) {
    if (seenFilenames.has(file)) {
      throw new Error(`duplicate migration filename ${file}`);
    }
    seenFilenames.add(file);

    const match = file.match(/^(\d{14})_.+\.sql$/);
    if (!match) {
      throw new Error(`bad migration filename ${file} (expected YYYYMMDDHHMMSS_name.sql)`);
    }

    const version = match[1];
    const existingFile = versionToFile.get(version);
    if (existingFile) {
      throw new Error(`duplicate migration version ${version}: ${existingFile}, ${file}`);
    }
    versionToFile.set(version, file);
  }

  return files;
}

function main() {
  const migrationsDir = process.argv[2] ? path.resolve(process.argv[2]) : defaultMigrationsDir();

  try {
    const files = checkMigrationFiles(migrationsDir);
    console.log(`supabase migrations ok (${files.length})`);
  } catch (error) {
    console.error(`error: ${error.message}`);
    process.exitCode = 1;
  }
}

if (require.main === module) {
  main();
}

module.exports = {
  checkMigrationFiles,
};
