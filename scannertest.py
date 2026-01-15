import re
import pytesseract
from PIL import Image
from PIL import ImageOps
import os
import logging
from watchdog.observers import Observer
from watchdog.events import FileSystemEventHandler
import time
import uuid
import shutil
import argparse
import sys

def check_dependencies(tesseract_path=None, tessdata_path=None):
    """Ensure necessary external tools are installed."""
    global TESSERACT_CMD

    if tesseract_path and tessdata_path:  # Allow user-specified paths during development/testing
        TESSERACT_CMD = tesseract_path
        tessdata_dir = tessdata_path
    elif getattr(sys, 'frozen', False):  # Running as a PyInstaller bundle
        base_path = sys._MEIPASS  # PyInstaller runtime path
        TESSERACT_CMD = os.path.join(base_path, "Tesseract-OCR", "tesseract.exe")
        tessdata_dir = os.path.join(base_path, "Tesseract-OCR", "tessdata")
    else:
        TESSERACT_CMD = shutil.which("tesseract")  # Use system-installed Tesseract if not frozen
        tessdata_dir = os.path.join(os.path.dirname(TESSERACT_CMD), "tessdata")

    # Log and exit if Tesseract is not found
    if TESSERACT_CMD is None or not os.path.isfile(TESSERACT_CMD):
        logging.error("Tesseract executable not found. Please install Tesseract or provide the correct path via arguments.")
        sys.exit(1)

    # Log and exit if tessdata directory doesn't exist
    if not os.path.exists(tessdata_dir):
        logging.error(f"'tessdata' directory not found at: {tessdata_dir}. Ensure Tesseract OCR is installed properly.")
        sys.exit(1)

    os.environ["TESSDATA_PREFIX"] = tessdata_dir
    pytesseract.pytesseract.tesseract_cmd = TESSERACT_CMD
    logging.info(f"Tesseract successfully loaded from: {TESSERACT_CMD}")
    
    

class POFileHandler(FileSystemEventHandler):
    def __init__(self, parent_directory, finished_directory, error_directory):
        self.parent_directory = parent_directory
        self.finished_directory = finished_directory
        self.error_directory = error_directory

    def on_created(self, event):
        if not event.is_directory:
            file_path = event.src_path
            logging.info(f"New file detected: {file_path}")
            time.sleep(3) #adding this pause, hoping this will resolve an access permission issue. 
            # Wait for the file to finish writing and be accessible
            while True:
                try:
                    with open(file_path, 'a'): #try to open the file in append mode
                        break
                        
                except IOError:
                    logging.info(f"Waiting for file access: {file_path}")
                    time.sleep(2) #this will wait one second and try again. 

            # Process PNG files only
            if file_path.lower().endswith('.png'):
                try:
                    po_number = extract_po_number_from_image(file_path)
                    
                    if po_number:
                        logging.info(f"Extracted PO Number: {po_number}")
                        self.rename_and_move(file_path, po_number)
                    
                    else:
                        logging.info("PO Number could not be extracted.")
                        self.handle_no_po_numbers(file_path)
                        
                except PermissionError as e:
                    logging.error(f"permission error while processing image {file_path}: {str(e)}")                    
                
            else:
                logging.warning(f"Unsupported file format detected: {file_path}")

    def rename_and_move(self, file_path, po_number):
        file_extension = os.path.splitext(file_path)[1]  # Get the file extension
        new_file_name = f"{po_number}_{uuid.uuid4().hex[:6]}{file_extension}"  # Create a unique name
        destination_path = os.path.join(self.finished_directory, new_file_name)

        # Ensure finished directory exists
        if not os.path.exists(self.finished_directory):
            os.makedirs(self.finished_directory)
            logging.info(f"Created finished directory: {self.finished_directory}")

        shutil.move(file_path, destination_path)  # Move file to finished directory
        logging.info(f"File renamed to '{new_file_name}' and moved to: {destination_path}")

    def handle_no_po_numbers(self, file_path):
        # Move files with no detectable PO numbers to an error directory
        if not os.path.exists(self.error_directory):
            os.makedirs(self.error_directory)
            logging.info(f"Created error directory: {self.error_directory}")

        error_file_path = os.path.join(self.error_directory, os.path.basename(file_path))
        shutil.move(file_path, error_file_path)
        logging.warning(f"File with no PO number moved to: {error_file_path}")

def extract_po_number_from_image(image_path):
    """
    Extracts the Purchase Order (PO) number from a PNG image using OCR.
    Adding rotation if it's not detecting a PO
    
    """
    try:
        # Open the image and preprocess
        file_access_tries = 5
        for tries in range(file_access_tries):
            try:
                img = Image.open(image_path)  # Attempt to open image
                img.load()  # Force-load the image data
                break
            except IOError:
                logging.info(f"Image '{image_path}' is locked. Retrying ({tries+1}/{file_access_tries})...")
                time.sleep(1)
        
        # If still unable to load the file, log and skip
        if tries == file_access_tries - 1:
            logging.error(f"Max retries reached. Skipping file: {image_path}")
            return None

        # Perform OCR to extract text
        ocr_result = pytesseract.image_to_string(img)
        logging.info(f"OCR raw output: {ocr_result}")
        po_number = re.search(r'[A-Z]*PO\d+', ocr_result)  # Adjusted regex for PPO numbers
        return po_number.group() if po_number else None
        
    except Exception as e:
        logging.error(f"Error processing image {image_path}: {str(e)}")
        return None
        
        
def get_PO_number_from_text(ocr_text):
    """
    Uses regex to extract the PO number from the OCR text.
    """
    
    
    po_number_match = re.search(r'[A-Z]*PO\d+', ocr_text)  # Look for "PO" followed by numbers using regex
    return po_number_match.group() if po_number_match else None
    
    

if __name__ == "__main__":
    # Parse command-line arguments
    parser = argparse.ArgumentParser(description="Monitor a directory for PO files.")
    parser.add_argument(
        "--root_directory",
        type=str,
        default=os.path.join(os.path.abspath(os.sep), "renamescans"),  # Default to "renamescans" in the root of the drive
        help="Root directory to house 'waves', 'wavesfinished', and 'UncapturedPO'. Default is root\\renamescans.",
    )
    args = parser.parse_args()

    # Define directories under the parent "renamescans" directory
    parent_directory = args.root_directory
    waves_directory = os.path.join(parent_directory, "waves")
    finished_directory = os.path.join(parent_directory, "wavesfinished")
    error_directory = os.path.join(finished_directory, "UncapturedPO")

    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s - [%(levelname)s] - %(message)s",
        datefmt="%Y-%m-%d %H:%M:%S"
    )

    check_dependencies()  # Check for Tesseract dependency

    # Ensure all directories exist
    for folder in [parent_directory, waves_directory, finished_directory, error_directory]:
        if not os.path.exists(folder):
            logging.info(f"Directory does not exist. Creating directory: {folder}")
            os.makedirs(folder)

    # Start monitoring the 'waves' directory
    event_handler = POFileHandler(parent_directory, finished_directory, error_directory)
    observer = Observer()
    observer.schedule(event_handler, path=waves_directory, recursive=False)

    try:
        logging.info(f"Monitoring directory: {waves_directory}")
        observer.start()  # Start monitoring
        while True:
            time.sleep(1)  # Keep the script running
    except KeyboardInterrupt:
        logging.info("Stopping directory monitoring.")
        observer.stop()
    observer.join()
