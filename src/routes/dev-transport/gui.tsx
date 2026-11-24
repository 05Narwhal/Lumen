import { Button } from "@mantine/core";
import { useState } from "react";
import { PageHeader } from "@/components/page-header";
import { pickFile } from "@/lib/dialogs";
import { invoke } from "@/lib/ipc";
import { mediaUrl } from "@/utils/media";
import "./styles.scss";

/** Debug-only proof that `photos://` honours Range: pick a video and seek. */
export function DevTransportRoute() {
  const [url, setUrl] = useState<string | null>(null);
  const pick = async () => {
    const file = await pickFile({
      filters: [{ name: "Video", extensions: ["mp4", "mov", "m4v", "webm"] }],
    });
    if (typeof file !== "string") return;
    const id = await invoke<number>("register_probe", { path: file });
    setUrl(mediaUrl("probe", id));
  };
  return (
    <div className="dev-transport">
      <PageHeader title="Transport check" search={false} toolbar={<Button onClick={pick}>Choose a video…</Button>} />
      <div className="dev-transport__body">
        {url ? (
          <>
            <code>{url}</code>
            <video controls src={url} className="dev-transport__video" />
          </>
        ) : (
          <p>Pick a video, then seek. If seeking works, Range support is proven.</p>
        )}
      </div>
    </div>
  );
}
