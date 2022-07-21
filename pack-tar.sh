#!/bin/bash

tar -zcvf ./liquidator.tar.gz \
	--exclude="./target" \
	--exclude="./solend-liquidator/target" \
	--exclude="./go-ethereum" \
	--exclude="./node_modules" \
	--exclude="./.git" \
	--exclude="./liquidator.tar.gz" \
	--exclude="./parity-common" .
