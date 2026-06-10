.PHONY: testbed-up testbed-down testbed-check

testbed-up:
	python3 testbed/run-all.py start

testbed-down:
	python3 testbed/run-all.py stop

testbed-check:
	python3 testbed/run-all.py check
