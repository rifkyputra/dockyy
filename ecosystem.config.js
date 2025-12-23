module.exports = {
  apps: [
    {
      name: 'dockyy-backend',
      cwd: './backend',
      script: '.venv/bin/python',
      args: ['-m', 'app'],
      env: {
        FLASK_ENV: 'development',
        FLASK_DEBUG: '1'
      },
      watch: false,
      autorestart: true,
      max_restarts: 10,
      min_uptime: '10s',
      error_file: './logs/backend-error.log',
      out_file: './logs/backend-out.log',
      log_date_format: 'YYYY-MM-DD HH:mm:ss Z'
    },
    {
      name: 'dockyy-frontend',
      cwd: './frontend',
      script: 'bun',
      args: ['run', 'dev'],
      env: {
        SERVER_URL: 'http://localhost:8012'
      },
      watch: false,
      autorestart: true,
      max_restarts: 10,
      min_uptime: '10s',
      error_file: './logs/frontend-error.log',
      out_file: './logs/frontend-out.log',
      log_date_format: 'YYYY-MM-DD HH:mm:ss Z'
    }
  ]
};
