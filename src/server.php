<?php

declare(strict_types=1);

const FRAUD_BODIES = [
    '{"approved":true,"fraud_score":0}',
    '{"approved":true,"fraud_score":0.2}',
    '{"approved":true,"fraud_score":0.4}',
    '{"approved":false,"fraud_score":0.6}',
    '{"approved":false,"fraud_score":0.8}',
    '{"approved":false,"fraud_score":1}',
];

function handle_request(\Swoole\Http\Request $req, \Swoole\Http\Response $resp): void {
    $uri = $req->server['request_uri'] ?? '';

    if ($uri === '/fraud-score') {
        $resp->end(FRAUD_BODIES[\rinha_fraud_count($req->rawContent() ?: '{}')]);
        return;
    }

    if ($uri === '/ready') {
        $resp->end('');
        return;
    }

    $resp->status(404);
    $resp->end('');
}

function build_server(string $sock): \Swoole\Http\Server {
    if (file_exists($sock)) {
        @unlink($sock);
    }

    $server = new \Swoole\Http\Server($sock, 0, SWOOLE_BASE, SWOOLE_SOCK_UNIX_STREAM);

    $server->set([
        'worker_num'                       => 1,
        'reactor_num'                      => 1,
        'tcp_fastopen'                     => true,
        'open_tcp_nodelay'                 => true,
        'tcp_defer_accept'                 => 1,
        'enable_coroutine'                 => false,
        'log_level'                        => SWOOLE_LOG_WARNING,
        'http_compression'                 => false,
        'send_yield'                       => false,
        'open_http2_protocol'              => false,
        'open_eof_check'                   => false,
        'open_length_check'                => false,
        'max_request'                      => 0,
        'max_conn'                         => 4096,
        'buffer_output_size'               => 2 * 1024 * 1024,
        'kernel_socket_recv_buffer_size'   => 65536,
        'kernel_socket_send_buffer_size'   => 65536,
    ]);

    $server->on('start', static function () use ($sock) {
        @chmod($sock, 0666);
    });

    $server->on('request', 'handle_request');
    return $server;
}

if (PHP_SAPI === 'cli' && !function_exists('opcache_get_status')) {
    fwrite(STDERR, "warning: opcache não habilitado\n");
}

if (!defined('PRELOADING')) {
    $sock = getenv('SOCK') ?: '/run/sock/api1.sock';
    build_server($sock)->start();
}
