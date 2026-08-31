import { google } from 'googleapis';
import * as cron from 'node-cron';
import { spawn } from 'child_process';
import * as dotenv from 'dotenv';
import * as path from 'path';

dotenv.config();

const GDRIVE_FOLDER_ID = process.env.GDRIVE_FOLDER_ID;
const SERVICE_ACCOUNT_FILE = process.env.GDRIVE_SERVICE_ACCOUNT_JSON || path.join(__dirname, '../service-account.json');
const KEEP_COUNT = parseInt(process.env.BACKUP_KEEP_COUNT || '2', 10);
const CRON_SCHEDULE = process.env.BACKUP_CRON || '0 2 * * *';

if (!GDRIVE_FOLDER_ID) {
  console.error('Missing GDRIVE_FOLDER_ID environment variable.');
  process.exit(1);
}

// Authenticate with Google Drive
const auth = new google.auth.GoogleAuth({
  keyFile: SERVICE_ACCOUNT_FILE,
  scopes: ['https://www.googleapis.com/auth/drive.file'],
});

const drive = google.drive({ version: 'v3', auth });

async function performBackup() {
  console.log(`[${new Date().toISOString()}] Starting database backup...`);

  const timestamp = new Date().toISOString().replace(/[:.]/g, '-');
  const filename = `craw-backup-${timestamp}.dump`;

  try {
    // 1. Spawn pg_dump
    // We use custom format (-Fc) which is compressed
    const pgDumpArgs = [
      '-h', process.env.DB_HOST || 'postgres',
      '-p', process.env.DB_PORT || '5432',
      '-U', process.env.POSTGRES_USER || 'crawler',
      '-Fc',
      process.env.POSTGRES_DB || 'craw'
    ];

    const pgDumpProcess = spawn('pg_dump', pgDumpArgs, {
      env: {
        ...process.env,
        PGPASSWORD: process.env.PG_PASSWORD
      }
    });

    pgDumpProcess.stderr.on('data', (data) => {
      console.error(`pg_dump stderr: ${data.toString()}`);
    });

    // 2. Stream to Google Drive
    console.log(`Uploading to Google Drive as ${filename}...`);
    const uploadRes = await drive.files.create({
      requestBody: {
        name: filename,
        parents: [GDRIVE_FOLDER_ID!],
      },
      media: {
        mimeType: 'application/octet-stream',
        body: pgDumpProcess.stdout
      }
    });

    console.log(`Upload complete! File ID: ${uploadRes.data.id}`);

    // Wait for pg_dump to exit
    await new Promise((resolve, reject) => {
      pgDumpProcess.on('close', (code) => {
        if (code === 0) resolve(true);
        else reject(new Error(`pg_dump exited with code ${code}`));
      });
      pgDumpProcess.on('error', reject);
    });

    console.log(`[${new Date().toISOString()}] Backup successful!`);

    // 3. Prune old backups
    await pruneOldBackups();

  } catch (error) {
    console.error(`[${new Date().toISOString()}] Backup failed:`, error);
  }
}

async function pruneOldBackups() {
  console.log(`Checking for old backups to prune (keeping ${KEEP_COUNT})...`);
  
  try {
    const res = await drive.files.list({
      q: `'${GDRIVE_FOLDER_ID}' in parents and trashed = false`,
      orderBy: 'createdTime desc',
      fields: 'files(id, name, createdTime)',
    });

    const files = res.data.files || [];
    console.log(`Found ${files.length} backups in the folder.`);

    if (files.length > KEEP_COUNT) {
      const filesToDelete = files.slice(KEEP_COUNT);
      for (const file of filesToDelete) {
        console.log(`Deleting old backup: ${file.name} (ID: ${file.id})`);
        await drive.files.delete({ fileId: file.id! });
      }
      console.log('Pruning complete.');
    } else {
      console.log('No backups to prune.');
    }
  } catch (error) {
    console.error('Failed to prune old backups:', error);
  }
}

console.log(`Backup service initialized. Schedule: ${CRON_SCHEDULE}`);

// Run immediately on startup
performBackup();

// Then schedule
cron.schedule(CRON_SCHEDULE, performBackup);
