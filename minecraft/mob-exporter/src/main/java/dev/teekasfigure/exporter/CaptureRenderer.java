package dev.teekasfigure.exporter;

import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.blaze3d.systems.VertexSorter;
import net.minecraft.client.MinecraftClient;
import net.minecraft.client.gl.SimpleFramebuffer;
import net.minecraft.client.render.entity.EntityRenderDispatcher;
import net.minecraft.client.texture.NativeImage;
import net.minecraft.client.util.math.MatrixStack;
import net.minecraft.entity.LivingEntity;
import net.minecraft.util.math.RotationAxis;
import org.joml.Matrix4fStack;
import org.joml.Matrix4f;
import org.joml.Quaternionf;
import org.joml.Vector3f;
import org.lwjgl.opengl.GL11;

/** Actual model rendering into RGBA/depth, without spawning anything in the world. */
public final class CaptureRenderer implements AutoCloseable {
    public enum View { TOP, FRONT }

    private final MinecraftClient client;
    private final SimpleFramebuffer framebuffer;
    private final int resolution;

    /** centerOther is Z for TOP and Y for FRONT; depth follows the viewing axis. */
    public record Capture(NativeImage image, double centerX, double centerOther,
                          double span, double minDepth, double depth) {}
    private record Bounds(int left, int top, int right, int bottom) {
        boolean clipped(int size) { return left < 2 || top < 2 || right >= size - 2 || bottom >= size - 2; }
    }

    public CaptureRenderer(MinecraftClient client, int resolution) {
        this.client = client;
        this.resolution = resolution;
        framebuffer = new SimpleFramebuffer(resolution, resolution, true, MinecraftClient.IS_SYSTEM_MAC);
        framebuffer.setClearColor(0, 0, 0, 0);
    }

    public Capture capture(LivingEntity entity, View captureView) {
        double span = Math.max(1.0, Math.max(entity.getWidth(), entity.getHeight())) * 4.0;
        double initialOther = captureView == View.TOP ? 0 : entity.getHeight() / 2.0;
        NativeImage image = null;
        Bounds bounds = null;
        for (int attempt = 0; attempt < 8; attempt++) {
            if (image != null) image.close();
            image = render(entity, span, 0, initialOther, captureView);
            try { bounds = bounds(image); } catch(Throwable error) { image.close(); throw error; }
            if (!bounds.clipped(resolution)) break;
            span *= 2;
        }
        if (bounds.clipped(resolution)) { image.close(); throw new IllegalStateException("Model does not fit capture"); }
        double centerX = (((bounds.left + bounds.right + 1) / 2.0) / resolution - 0.5) * span;
        double pixelCenterY = (bounds.top + bounds.bottom + 1) / 2.0;
        double centerOther = captureView == View.TOP
            ? (pixelCenterY / resolution - 0.5) * span
            : initialOther + (0.5 - pixelCenterY / resolution) * span;
        double fittedSpan = Math.max(bounds.right - bounds.left + 1, bounds.bottom - bounds.top + 1)
            * span / resolution * 1.15;
        image.close();
        image = render(entity, fittedSpan, centerX, centerOther, captureView);
        try {
            bounds = bounds(image);
            if (bounds.clipped(resolution)) throw new IllegalStateException("Fitted model is clipped");
            // Measure the silhouette along the viewing axis for reliable world
            // layering instead of trusting hitboxes around horns or wings.
            double measureSpan = span;
            double measureCenter = captureView == View.TOP ? entity.getHeight() / 2.0 : 0;
            double minDepth = captureView == View.TOP ? 0 : -entity.getWidth() / 2.0;
            double maxDepth = captureView == View.TOP ? entity.getHeight() : entity.getWidth() / 2.0;
            for (int attempt = 0; attempt < 8; attempt++) {
                View measuringView = captureView == View.TOP ? View.FRONT : View.TOP;
                try (NativeImage measured = render(entity, measureSpan, centerX, measureCenter, measuringView)) {
                    Bounds b = bounds(measured);
                    if (!b.clipped(resolution)) {
                        if (captureView == View.TOP) {
                            minDepth = Math.min(0, measureCenter + (0.5 - (b.bottom + 1.0) / resolution) * measureSpan) - 0.05;
                            maxDepth = Math.max(entity.getHeight(), measureCenter + (0.5 - b.top / (double)resolution) * measureSpan) + 0.15;
                        } else {
                            minDepth = Math.min(minDepth, measureCenter + (b.top / (double)resolution - 0.5) * measureSpan) - 0.05;
                            maxDepth = Math.max(maxDepth, measureCenter + ((b.bottom + 1.0) / resolution - 0.5) * measureSpan) + 0.05;
                        }
                        break;
                    }
                }
                measureSpan *= 2;
                if (attempt == 7) throw new IllegalStateException("Cannot measure model height");
            }
            unpremultiply(image);
            return new Capture(image, centerX, centerOther, fittedSpan, minDepth, (maxDepth - minDepth) * 1.10);
        } catch (Throwable error) { image.close(); throw error; }
    }

    private NativeImage render(LivingEntity entity, double span, double centerX, double centerOther, View captureView) {
        var dispatcher = client.getEntityRenderDispatcher();
        var vertices = client.getBufferBuilders().getEntityVertexConsumers();
        vertices.draw();
        boolean depth = GL11.glIsEnabled(GL11.GL_DEPTH_TEST);
        boolean blend = GL11.glIsEnabled(GL11.GL_BLEND);
        boolean cull = GL11.glIsEnabled(GL11.GL_CULL_FACE);
        Quaternionf oldRotation = new Quaternionf(dispatcher.getRotation());
        RenderSystem.backupProjectionMatrix();
        Matrix4fStack view = RenderSystem.getModelViewStack();
        view.pushMatrix();
        view.identity();
        RenderSystem.applyModelViewMatrix();
        try {
            framebuffer.clear(MinecraftClient.IS_SYSTEM_MAC);
            framebuffer.beginWrite(true);
            RenderSystem.setProjectionMatrix(new Matrix4f().setOrtho(
                (float)-span/2, (float)span/2, (float)-span/2, (float)span/2, -2048, 2048), VertexSorter.BY_Z);
            RenderSystem.enableDepthTest();
            RenderSystem.enableCull();
            RenderSystem.enableBlend();
            RenderSystem.defaultBlendFunc();
            RenderSystem.setShaderColor(1,1,1,1);
            // Match the fixed world-playback light to the selected camera pitch.
            Vector3f light = captureView == View.TOP ? new Vector3f(0,0,1) : new Vector3f(0,1,0);
            RenderSystem.setShaderLights(light, light);
            dispatcher.setRotation(new Quaternionf().rotationX(captureView == View.TOP ? (float)-Math.PI/2 : 0));
            MatrixStack matrices = new MatrixStack();
            if (captureView == View.TOP) {
                matrices.multiply(RotationAxis.POSITIVE_X.rotationDegrees(90));
                matrices.translate(-centerX, 0, -centerOther);
            } else matrices.translate(-centerX, -centerOther, 0);
            dispatcher.getRenderer(entity).render(entity, entity.getYaw(), 0, matrices, vertices, 0xF000F0);
            vertices.draw();
            framebuffer.beginRead();
            NativeImage image = new NativeImage(resolution, resolution, false);
            try {
                image.loadFromTextureImage(0, false);
                image.mirrorVertically();
                return image;
            } catch (Throwable error) { image.close(); throw error; }
            finally { framebuffer.endRead(); }
        } finally {
            dispatcher.setRotation(oldRotation);
            view.popMatrix();
            RenderSystem.applyModelViewMatrix();
            RenderSystem.restoreProjectionMatrix();
            client.getFramebuffer().beginWrite(true);
            RenderSystem.setShaderColor(1,1,1,1);
            if (depth) RenderSystem.enableDepthTest(); else RenderSystem.disableDepthTest();
            if (blend) RenderSystem.enableBlend(); else RenderSystem.disableBlend();
            if (cull) RenderSystem.enableCull(); else RenderSystem.disableCull();
            net.minecraft.client.render.DiffuseLighting.enableGuiDepthLighting();
        }
    }

    private Bounds bounds(NativeImage image) {
        int minX = resolution, minY = resolution, maxX = -1, maxY = -1;
        for (int y=0; y<resolution; y++) for (int x=0; x<resolution; x++) {
            if ((image.getColor(x,y) >>> 24) != 0) {
                minX=Math.min(minX,x); minY=Math.min(minY,y);
                maxX=Math.max(maxX,x); maxY=Math.max(maxY,y);
            }
        }
        if (maxX < 0) throw new IllegalStateException("Model rendered no visible pixels");
        return new Bounds(minX,minY,maxX,maxY);
    }

    private static void unpremultiply(NativeImage image) {
        for (int y=0;y<image.getHeight();y++) for(int x=0;x<image.getWidth();x++) {
            int c=image.getColor(x,y), a=c>>>24;
            if (a>0 && a<255) {
                int r=Math.min(255,(c&255)*255/a), g=Math.min(255,((c>>>8)&255)*255/a), b=Math.min(255,((c>>>16)&255)*255/a);
                image.setColor(x,y,(a<<24)|(b<<16)|(g<<8)|r);
            }
        }
    }

    @Override public void close() { framebuffer.delete(); }
}

