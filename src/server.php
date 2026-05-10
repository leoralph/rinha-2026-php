<?php

declare(strict_types=1);

function run_server(string $sock): void {
    if (file_exists($sock)) {
        @unlink($sock);
    }

    $listener = new \Swoole\Coroutine\Socket(AF_UNIX, SOCK_STREAM, 0);
    if (!$listener->bind($sock)) {
        fwrite(STDERR, "bind failed: " . $listener->errMsg . "\n");
        exit(1);
    }
    @chmod($sock, 0666);
    if (!$listener->listen(4096)) {
        fwrite(STDERR, "listen failed: " . $listener->errMsg . "\n");
        exit(1);
    }

    while (true) {
        $client = $listener->accept(-1);
        if ($client === false) {
            continue;
        }
        \Swoole\Coroutine::create(static function () use ($client): void {
            $fd = (int) $client->fd;
            try {
                while (true) {
                    $data = $client->recv(8192, -1);
                    if ($data === false || $data === '') {
                        break;
                    }
                    $resp = \rinha_handle_batch($fd, $data);
                    if ($resp !== '') {
                        $client->send($resp);
                    }
                }
            } finally {
                \rinha_close($fd);
                $client->close();
            }
        });
    }
}

if (!defined('PRELOADING')) {
    \Swoole\Coroutine\run(static function (): void {
        $sock = getenv('SOCK') ?: '/run/sock/api1.sock';
        run_server($sock);
    });
}
