<?php

namespace App\Store;

/** Reads rows by id. */
interface Repository
{
    public function find(int $id): ?Row;
}

trait Checks
{
    protected function check(int $id): void
    {
    }
}

class MemoryRepository implements Repository
{
    use Checks;

    private array $rows = [];

    public function find(int $id): ?Row
    {
        $this->check($id);
        return $this->rows[$id] ?? null;
    }

    public function put(Row $row): void
    {
        $this->rows[$row->id] = $row;
    }
}
