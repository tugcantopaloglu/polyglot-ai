#!/bin/bash
#
# Seamless Update Script for Polyglot-AI Server
# Performs zero-downtime updates using graceful shutdown
#
# Made by Tugcan Topaloglu

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
DOCKER_DIR="$PROJECT_ROOT/docker"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${CYAN}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

print_banner() {
    echo -e "${CYAN}"
    echo "  ____       _             _       _        _    ___ "
    echo " |  _ \ ___ | |_   _  __ _| | ___ | |_     / \  |_ _|"
    echo " | |_) / _ \| | | | |/ _\` | |/ _ \| __|   / _ \  | | "
    echo " |  __/ (_) | | |_| | (_| | | (_) | |_   / ___ \ | | "
    echo " |_|   \___/|_|\__, |\__, |_|\___/ \__| /_/   \_\___|"
    echo "               |___/ |___/                           "
    echo -e "${NC}"
    echo "       Seamless Update Script - Zero Downtime"
    echo ""
}

check_docker() {
    if ! command -v docker &> /dev/null; then
        log_error "Docker is not installed"
        exit 1
    fi

    if ! docker info &> /dev/null; then
        log_error "Docker daemon is not running"
        exit 1
    fi
}

check_compose() {
    if command -v docker-compose &> /dev/null; then
        COMPOSE_CMD="docker-compose"
    elif docker compose version &> /dev/null; then
        COMPOSE_CMD="docker compose"
    else
        log_error "Docker Compose is not installed"
        exit 1
    fi
}

get_current_version() {
    if docker inspect polyglot-server &> /dev/null 2>&1; then
        docker inspect polyglot-server --format '{{.Config.Labels.version}}' 2>/dev/null || echo "unknown"
    else
        echo "not running"
    fi
}

pull_latest() {
    log_info "Pulling latest changes..."
    cd "$PROJECT_ROOT"
    git fetch origin main
    git pull origin main
}

build_new_image() {
    log_info "Building new server image..."
    cd "$DOCKER_DIR"
    $COMPOSE_CMD build --no-cache polyglot-server
    log_success "New image built successfully"
}

graceful_shutdown() {
    local timeout=${1:-30}
    log_info "Initiating graceful shutdown (${timeout}s timeout)..."
    
    # Send SIGTERM for graceful shutdown
    if docker ps --format '{{.Names}}' | grep -q "polyglot-server"; then
        docker kill --signal=SIGTERM polyglot-server 2>/dev/null || true
        
        # Wait for connections to drain
        local count=0
        while [ $count -lt $timeout ]; do
            if ! docker ps --format '{{.Names}}' | grep -q "polyglot-server"; then
                break
            fi
            sleep 1
            ((count++))
            echo -ne "\r   Waiting for connections to drain... ${count}/${timeout}s"
        done
        echo ""
        
        if docker ps --format '{{.Names}}' | grep -q "polyglot-server"; then
            log_warning "Graceful shutdown timeout, forcing stop..."
            docker stop polyglot-server
        fi
    fi
}

start_new_container() {
    log_info "Starting new server container..."
    cd "$DOCKER_DIR"
    $COMPOSE_CMD up -d polyglot-server
    
    # Wait for health check
    log_info "Waiting for server to become healthy..."
    local count=0
    local max_wait=60
    
    while [ $count -lt $max_wait ]; do
        local health=$(docker inspect --format='{{.State.Health.Status}}' polyglot-server 2>/dev/null || echo "starting")
        
        if [ "$health" = "healthy" ]; then
            log_success "Server is healthy and ready!"
            return 0
        fi
        
        sleep 2
        ((count+=2))
        echo -ne "\r   Health check... ${count}/${max_wait}s (status: $health)"
    done
    
    echo ""
    log_error "Server failed to become healthy within ${max_wait}s"
    return 1
}

rollback() {
    log_warning "Rolling back to previous version..."
    cd "$DOCKER_DIR"
    
    # Get previous image
    local prev_image=$(docker images --format "{{.Repository}}:{{.Tag}}" | grep polyglot-server | head -2 | tail -1)
    
    if [ -n "$prev_image" ]; then
        docker tag "$prev_image" polyglot-server:rollback
        $COMPOSE_CMD up -d polyglot-server
        log_success "Rollback completed"
    else
        log_error "No previous image found for rollback"
        exit 1
    fi
}

cleanup_old_images() {
    log_info "Cleaning up old images..."
    docker image prune -f --filter "label=app=polyglot-server" --filter "until=24h" 2>/dev/null || true
    log_success "Cleanup completed"
}

update_binary() {
    log_info "Updating binary installation..."
    cd "$PROJECT_ROOT"
    
    # Build release binaries
    cargo build --release
    
    # Backup current binaries
    if [ -f /usr/local/bin/polyglot-server ]; then
        cp /usr/local/bin/polyglot-server /usr/local/bin/polyglot-server.bak
    fi
    
    # Install new binaries
    cp target/release/polyglot-server /usr/local/bin/
    cp target/release/polyglot /usr/local/bin/
    cp target/release/polyglot-local /usr/local/bin/
    
    log_success "Binaries updated"
}

restart_systemd_service() {
    if systemctl is-active --quiet polyglot-server; then
        log_info "Restarting systemd service with graceful reload..."
        systemctl reload-or-restart polyglot-server
        log_success "Service restarted"
    else
        log_warning "Systemd service not active, starting..."
        systemctl start polyglot-server
    fi
}

# Main execution
main() {
    print_banner
    
    local update_type="docker"
    local skip_pull=false
    local graceful_timeout=30
    
    while [[ $# -gt 0 ]]; do
        case $1 in
            --binary)
                update_type="binary"
                shift
                ;;
            --docker)
                update_type="docker"
                shift
                ;;
            --skip-pull)
                skip_pull=true
                shift
                ;;
            --timeout)
                graceful_timeout="$2"
                shift 2
                ;;
            --rollback)
                rollback
                exit 0
                ;;
            --help)
                echo "Usage: $0 [OPTIONS]"
                echo ""
                echo "Options:"
                echo "  --docker      Update Docker container (default)"
                echo "  --binary      Update binary installation"
                echo "  --skip-pull   Skip git pull"
                echo "  --timeout N   Graceful shutdown timeout in seconds (default: 30)"
                echo "  --rollback    Rollback to previous version"
                echo "  --help        Show this help"
                exit 0
                ;;
            *)
                log_error "Unknown option: $1"
                exit 1
                ;;
        esac
    done
    
    local current_version=$(get_current_version)
    log_info "Current version: $current_version"
    
    if [ "$skip_pull" = false ]; then
        pull_latest
    fi
    
    if [ "$update_type" = "docker" ]; then
        check_docker
        check_compose
        
        build_new_image
        graceful_shutdown "$graceful_timeout"
        
        if start_new_container; then
            cleanup_old_images
            log_success "Update completed successfully!"
        else
            log_error "Update failed, initiating rollback..."
            rollback
            exit 1
        fi
    else
        update_binary
        restart_systemd_service
        log_success "Binary update completed!"
    fi
    
    local new_version=$(get_current_version)
    log_info "New version: $new_version"
}

main "$@"
