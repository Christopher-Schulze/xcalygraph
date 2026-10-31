<?php

namespace App\Http;

use App\Store\{MemoryRepository, Row};
use App\Store\Repository as Repo;

class Controller
{
    public function __construct(private Repo $repository) {}

    public function show(int $id): string
    {
        $row = $this->repository->find($id);
        return format_row($row) . getenv('APP_URL');
    }

    public static function make(): self
    {
        $repository = new MemoryRepository();
        $repository->put(new Row(1));
        return new Controller($repository);
    }
}

function format_row(?Row $row): string
{
    return (string) $row?->id;
}
