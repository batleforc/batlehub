<?php

declare(strict_types=1);

namespace App;

use Symfony\Component\Console\Application;
use Symfony\Component\Console\Command\Command;
use Symfony\Component\Console\Input\InputInterface;
use Symfony\Component\Console\Output\OutputInterface;

class HelloCommand extends Command
{
    protected static string $defaultName = 'app:hello';

    protected function execute(InputInterface $input, OutputInterface $output): int
    {
        $output->writeln('<info>Hello from my-app!</info>');
        return Command::SUCCESS;
    }
}

$app = new Application('my-app', '1.0.0');
$app->add(new HelloCommand());
$app->run();
