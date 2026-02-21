#!/usr/bin/env node
'use strict';

const { createServer } = require('./server');

const args = process.argv.slice(2);
let port = 16726;

for (let i = 0; i < args.length; i++) {
  if (args[i] === '--port' && args[i + 1]) {
    port = parseInt(args[i + 1], 10);
    if (Number.isNaN(port) || port < 1 || port > 65535) {
      console.error('Invalid port number: ' + args[i + 1]);
      process.exit(1);
    }
    i++;
  } else if (args[i] === '--help' || args[i] === '-h') {
    console.log('Usage: kalatori-webhook-simulator [--port PORT]');
    console.log('');
    console.log('Options:');
    console.log('  --port PORT  Port to listen on (default: 16726)');
    process.exit(0);
  }
}

const server = createServer();

server.listen(port, '127.0.0.1', () => {
  const url = 'http://localhost:' + port;
  console.log('Kalatori Webhook Simulator running at ' + url);
  console.log('Press Ctrl+C to stop.\n');

  // Open browser automatically
  const { exec } = require('child_process');
  const platform = process.platform;
  const cmd = platform === 'darwin' ? 'open'
    : platform === 'win32' ? 'start'
    : 'xdg-open';
  exec(cmd + ' ' + url, () => {
    // Ignore errors — browser may not be available (e.g. headless server)
  });
});

process.on('SIGINT', () => {
  console.log('\nShutting down...');
  server.close(() => process.exit(0));
});

process.on('SIGTERM', () => {
  server.close(() => process.exit(0));
});
