FROM lscr.io/linuxserver/blender@sha256:d66c5758fb64316bcc88a3e779b2d544df5c1d8c320f8003444994906657f230

RUN curl --fail --show-error --location \
      https://files.pythonhosted.org/packages/47/bf/60081137d09cb5a16195d0713fd46af711f27a594750244ec12722f23adf/mcp_for_blender-2.1.9.tar.gz \
      --output /tmp/mcp-for-blender.tar.gz \
    && echo "c25f59b9598a85ec633ce93cf155fbf80437b58441de793d610cb942a343dbd9  /tmp/mcp-for-blender.tar.gz" | sha256sum --check --strict \
    && mkdir -p /opt/tks-blender \
    && tar -xOzf /tmp/mcp-for-blender.tar.gz \
      mcp_for_blender-2.1.9/src/blender_mcp/bundled/addon.py \
      > /opt/tks-blender/blender_mcp.py \
    && rm /tmp/mcp-for-blender.tar.gz

COPY extensions.lock.json fetch_extensions.py /opt/tks-blender/
RUN python3 /opt/tks-blender/fetch_extensions.py \
      /opt/tks-blender/extensions.lock.json /opt/tks-blender/extensions

COPY bootstrap.py launch-blender.sh check_ready.py /opt/tks-blender/
RUN chmod 755 /opt/tks-blender/launch-blender.sh \
    && printf '%s\n' '/opt/tks-blender/launch-blender.sh &' > /defaults/autostart \
    && cp /defaults/autostart /defaults/autostart_wayland
