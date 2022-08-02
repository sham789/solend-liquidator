#!/bin/bash


tar -zcvf ./solend-liquidator.tar.gz \
	--exclude="./solend-liquidator/target" \
	--exclude="./solend-liquidator/private" \
	--exclude="./solend-liquidator-benchmarking/target" \
	--exclude="./solend-liquidator.tar.gz" \
	--exclude="./solana-program-library/target" \
	--exclude="./node_modules" \
	--exclude="./overriden/anchor/target" \
	--exclude="./overriden/anchor/tests" \
	--exclude="./overriden/switchboard-program-0.2.1/target" \
	--exclude="./overriden/switchboard-program-0.2.1/tests" \
	--exclude="./overriden/switchboard-v2-0.1.10/target" \
	--exclude="./overriden/switchboard-v2-0.1.10/tests" \
	--exclude="./.git" .
