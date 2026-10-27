# Historical unsafe host assertion from edf65bd7; regression fixture only.
        if [ "$HTTP_TEST" = "1" ] && [ "$HTTP_PROBED" = "0" ] \
            && grep -Fq "httpd is listening" "$LOG"; then
            HTTP_PROBED=1
            echo ">>> Probing guest node and cluster keyholes through port ${HTTP_HOST_PORT} ..."
            DEPLOY_LIVENESS_OK=0
            if python3 "${ROOT_DIR}/scripts/tests/test-http-keyhole-liveness.py" \
                --port "$HTTP_HOST_PORT" --ready-log "$LOG" --wait-seconds 1 \
                --require-dns-catalog --require-ns-status \
                && python3 "${ROOT_DIR}/scripts/tests/test-deployment-http-liveness.py" \
                    --port "$DEPLOY_HOST_PORT" --ready-log "$LOG"; then
                DEPLOY_LIVENESS_OK=1
            fi
            if [ -n "${CATTEN_MANAGEMENT_READER_CERT_FILE:-}" ] \
                && [ -n "${CATTEN_MANAGEMENT_UNKNOWN_CERT_FILE:-}" ]; then
                if ! python3 "${ROOT_DIR}/scripts/tests/test-management-tls.py" \
                    --status-port "$HTTP_HOST_PORT" --deployment-port "$DEPLOY_HOST_PORT"; then
                    HTTP_LIVENESS_OK=0
                fi
            fi
            for _ in 1 2 3 4 5 6 7 8; do
                HTTP_BODY="$(curl -fsS --max-time 5 --cacert "$CATTEN_MANAGEMENT_CA_FILE" --cert "$CATTEN_MANAGEMENT_CERT_FILE" --key "$CATTEN_MANAGEMENT_KEY_FILE" https://127.0.0.1:${HTTP_HOST_PORT}/metrics 2>&1 || true)"
                HTTP_CLUSTER_BODY="$(curl -fsS --max-time 5 --cacert "$CATTEN_MANAGEMENT_CA_FILE" --cert "$CATTEN_MANAGEMENT_CERT_FILE" --key "$CATTEN_MANAGEMENT_KEY_FILE" https://127.0.0.1:${HTTP_HOST_PORT}/cluster/metrics 2>&1 || true)"
                if [ "$DEPLOY_LIVENESS_OK" = "1" ] \
                    && printf '%s' "$HTTP_BODY" | grep -Fq '"http":{"requests":' \
                    && printf '%s' "$HTTP_CLUSTER_BODY" | python3 -c \
                        'import json,sys; d=json.load(sys.stdin); assert d["schema"] == "charlotte.cluster.v2" and d["fresh_committed"] and d["raft"]["served_by"] and all("object_id" not in e for e in d["deployments"])'; then
                    HTTP_PROBE_OK=1
                    break
                fi
                sleep 2
            done
            echo ">>> Guest node keyhole response:"
            echo "$HTTP_BODY"
            echo ">>> Guest cluster keyhole response:"
            echo "$HTTP_CLUSTER_BODY"
        fi
