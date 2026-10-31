<?php

use App\Store\MemoryRepository;
use App\Store\Row;
use PHPUnit\Framework\TestCase;

class RepositoryTest extends TestCase
{
    public function testFindsStoredRow(): void
    {
        $repository = new MemoryRepository();
        $repository->put(new Row(1));
        $this->assertNotNull($repository->find(1));
    }
}
