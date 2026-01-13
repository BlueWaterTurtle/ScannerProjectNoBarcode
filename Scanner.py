#TODO: I'm toying with the idea of creating a second script to handle backups and deleting the working directories of this script. 



# Program for monitoring a folder and renaming any new files that enter the directory
import os
import time
import shutil
from watchdog.observers import Observer
from watchdog.events import FileSystemEventHandler
from pyzbar.pyzbar import decode
from PIL import Image
import fitz  # PyMuPDF
import io

class MyHandler(FileSystemEventHandler):
    def on_created(self, event):
        time.sleep(4)         #I had to add a small delay at the beginning, the scanner may have been preventing the script from running, so I added the delay to allow the scanner to finish with the file before this script takes over.
        print(f"New file created: {event.src_path}")
        
        # paths 
        starter_path = event.src_path # this is the directory where the physicall scanner will send the .png files to start
        destination_dir = 'C:\\Users\\Public\\Documents\\ProcessedWaves'  # Define the destination directory, change it as needed. 
        finished_dir = 'C:\\Users\\Public\\Documents\\WavesFinished'  # Define the finished directory, change as needed. 
        os.makedirs(destination_dir, exist_ok=True)  # Create the directory if it doesn't exist
        os.makedirs(finished_dir, exist_ok=True)  # Create the finished directory if it doesn't exist

        # Extract barcode data from the file
        barcode_data = self.extract_barcode(starter_path)
        if barcode_data:
            # Define the new name and path
            file_extension = os.path.splitext(starter_path)[1]
            new_name = f"{barcode_data}{file_extension}"
            destination_path = os.path.join(destination_dir, new_name)

            # Check if file already exists and append a unique identifier if necessary, using a time stamp because these are added to a sharepoint by the end user. if there are name conflicts, they'll get overwritten without warning. 
            if os.path.exists(destination_path):
                base_name = os.path.splitext(new_name)[0]
                timestamp = time.strftime("%Y%m%d%H%M%S")
                new_name = f"{base_name}_{timestamp}{file_extension}"
                destination_path = os.path.join(destination_dir, new_name)

            # Copy and rename the file to the destination directory
            shutil.copy(starter_path, destination_path)
            print(f"File copied and renamed to: {destination_path}")

            # Convert PNG to PDF if necessary and move to finished directory
            if file_extension.lower() == '.png':
                pdf_path = self.convert_png_to_pdf(destination_path)
                if pdf_path:
                    print(f"PNG converted to PDF: {pdf_path}")
                    self.move_to_finished(pdf_path, finished_dir)
                    #adding cleanup here. 
                    cleanup_files([pdf_path, destination_path])                    
            elif file_extension.lower() == '.pdf':
                self.move_to_finished(destination_path, finished_dir)
                #adding cleanup here too
                cleanup_files([destination_path])
        else:
            print("No barcode detected in the file.")
        #cleaning up starter directory
        cleanup_files([starter_path])

    def extract_barcode(self, file_path):
        try:
            file_extension = os.path.splitext(file_path)[1].lower()
            if file_extension == '.pdf':
                return self.extract_barcode_from_pdf(file_path)
            else:
                return self.extract_barcode_from_image(file_path)
        except Exception as e:
            print(f"Error extracting barcode: {e}")
            return None

    def extract_barcode_from_image(self, file_path):
        try:
            img = Image.open(file_path)
            decoded_objects = decode(img)
            if decoded_objects:
                return decoded_objects[0].data.decode('utf-8')
            else:
                return None
        except Exception as e:
            print(f"Error extracting barcode from image: {e}")
            return None
        finally:
            img.close()

    def extract_barcode_from_pdf(self, file_path):
        try:
            doc = fitz.open(file_path)
            for page_num in range(len(doc)):
                page = doc.load_page(page_num)
                pix = page.get_pixmap()
                img = Image.open(io.BytesIO(pix.tobytes()))
                decoded_objects = decode(img)
                if decoded_objects:
                    return decoded_objects[0].data.decode('utf-8')
            return None
        except Exception as e:
            print(f"Error extracting barcode from PDF: {e}")
            return None
        finally:
            doc.close()

    def convert_png_to_pdf(self, png_path):
        try:
            img = Image.open(png_path)
            pdf_path = os.path.splitext(png_path)[0] + '.pdf'
            img.save(pdf_path, 'PDF', resolution=100.0)
            return pdf_path
        except Exception as e:
            print(f"Error converting PNG to PDF: {e}")
            return None

    def move_to_finished(self, file_path, finished_dir):
        try:
            shutil.move(file_path, finished_dir)
            print(f"File moved to: {os.path.join(finished_dir, os.path.basename(file_path))}")
        except Exception as e:
            print(f"Error moving file to finished directory: {e}")
            
def cleanup_files(file_paths):
    """Clean specific files after processing."""
    print("Performing cleanup of specific files...")
    time.sleep(2)  # Add a small delay before cleanup.
    for file_path in file_paths:
        try:
            if os.path.exists(file_path):
                os.remove(file_path)
                print(f"Deleted file: {file_path}")
            else: 
                print(f" File not found, skipping: {file_path}")
        except Exception as e:
            print(f"Error deleting file {file_path}: {e}")

def main():
    path = 'C:\\Users\\Public\\Documents\\Waves'  # this is the directory where the scanner should be configured to send .png's on scan. change it as needed. 
    os.makedirs(path, exist_ok=True)  # Create the directory if it doesn't exist
    event_handler = MyHandler()
    observer = Observer()
    observer.schedule(event_handler, path, recursive=True)
    observer.start()
    print("Observer started. Monitoring directory for changes...")

    try:
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        print("Keyboard interrupt received. Stopping observer...")
        observer.stop()
    observer.join()
    print("Observer stopped.")

if __name__ == "__main__":
    main()
    input("Press Enter to exit...")
