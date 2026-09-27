package dev.teekasfigure.exporter;

import net.minecraft.text.Text;

/** Cache parsed ownership/pose/scale until the tracked name object changes. */
public interface ParticipantCache {
    Participant.Info teekasfigure$metadata(Text name);
}
