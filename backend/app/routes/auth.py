from flask import Blueprint, request, jsonify
import jwt
import datetime
import os

auth_bp = Blueprint('auth', __name__)

DEFAULT_ADMIN_USERNAME = os.environ.get('DEFAULT_ADMIN_USERNAME', 'admin')
DEFAULT_ADMIN_PASSWORD = os.environ.get('DEFAULT_ADMIN_PASSWORD', 'adminpass')
# Simple in-memory user store for demo (replace with real database)
users = {
    DEFAULT_ADMIN_USERNAME: {'password': DEFAULT_ADMIN_PASSWORD, 'role': 'admin'}
}

SECRET_KEY = os.environ.get('SECRET_KEY', 'your-secret-key')

@auth_bp.route('/login', methods=['POST'])
def login():
    print("Login attempt received")
    print(f"Request data: {request.get_json()}")
    print(f"users: {users.get(request.get_json().get('username'))}")
    data = request.get_json()
    username = data.get('username')
    password = data.get('password')

    user = users.get(username)
    if user and user['password'] == password:
        token = jwt.encode({
            'username': username,
            'role': user['role'],
            'exp': datetime.datetime.utcnow() + datetime.timedelta(hours=24)
        }, SECRET_KEY, algorithm='HS256')
        return jsonify({'token': token}), 200
    return jsonify({'error': 'Invalid credentials'}), 401

@auth_bp.route('/logout', methods=['POST'])
def logout():
    # For JWT, logout is handled client-side by removing the token
    return jsonify({'message': 'Logged out'}), 200

def init_routes(app):
    app.register_blueprint(auth_bp, url_prefix='/api/auth')