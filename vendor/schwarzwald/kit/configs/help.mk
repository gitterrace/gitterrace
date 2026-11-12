# The one `make help` every product prints. A product includes it from its copy, last:
#
#   include vendor/schwarzwald/kit/configs/help.mk
#
# and documents its Makefile with two kinds of comment line:
#
#   ##@ Section title            starts a group in the list
#   ## target: one line          a target a person runs, listed under the section above it
#
# Plain `#` lines right above a `##` line are its longer story. A target with no `##` line is
# an internal step (one `check` composes) and stays out of the list. `make help` prints the
# groups. `make help TARGET=x` prints that target's line, its story and its recipe, so a reader
# sees what it runs. The make-help lint keeps every `##` line pointing at a real target, and
# every target of the product's [make] core list present and documented.

.DEFAULT_GOAL := help
.PHONY: help

##@ Help
## help: This list. TARGET=<name> explains one target and shows what it runs
help:
	@awk -v want="$(TARGET)" -v files="$(words $(MAKEFILE_LIST))" ' \
		FNR == 1 { seen++ } \
		seen <= files && /^## [A-Za-z0-9_.-]+: / { t = $$2; sub(/:$$/, "", t); \
			if (length(t) > w) w = length(t); next } \
		seen <= files { next } \
		/^##@ / { if (want == "") printf "\n%s\n", substr($$0, 5); n = 0; next } \
		/^## [A-Za-z0-9_.-]+: / { \
			t = $$2; sub(/:$$/, "", t); d = $$0; sub(/^## [^ ]+ /, "", d); \
			if (want == "") printf "  %-*s  %s\n", w, t, d; \
			else if (t == want) { found = 1; print t ": " d; \
				for (i = 1; i <= n; i++) print "  " substr(story[i], 3); grab = 1 } \
			n = 0; next } \
		/^# / { story[++n] = $$0; next } \
		grab && $$0 ~ "^" want ":" { inrule = 1; print ""; print $$0; next } \
		inrule && /^\t/ { print $$0; next } \
		inrule { exit } \
		{ n = 0 } \
		END { if (want != "" && !found) { print "no documented target " want > "/dev/stderr"; exit 1 } \
			if (want == "") print "" } \
	' $(MAKEFILE_LIST) $(MAKEFILE_LIST)
