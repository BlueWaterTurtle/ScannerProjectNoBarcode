#legacy, just here for reference. use the other. 


import re
import pytesseract
from pytesseract import Output
from pdf2image import convert_from_path
from PIL import Image
import os
import logging

def extract_po_number_from_image(image_path):
    """
    Extracts the Purchase Order (PO) number from a PNG image using OCR.
    """
    try:
        # Use Tesseract OCR to extract text from the image
        ocr_result = pytesseract.image_to_string(Image.open(image_path))

        # Use regex to search for PO patterns, e.g., 'PO12345'
        po_number_match = re.search(r'PO\d+', ocr_result)

        if po_number_match:
            logging.info(f"PO Number '{po_number_match.group()}' found in image: {image_path}")
            return po_number_match.group()
        else:
            logging.warning(f"No PO Number found in image: {image_path}")
            return None
    except Exception as e:
        logging.error(f"Error processing image {image_path}: {str(e)}")
        return None

def extract_po_number_from_pdf(pdf_path):
    """
    Extracts the Purchase Order (PO) number from a PDF file using OCR.
    """
    try:
        # Convert PDF pages to images
        pages = convert_from_path(pdf_path)
        for page_number, page in enumerate(pages, start=1):
            # Save page as a temporary PNG image
            temp_image_path = f"temp_page_{page_number}.png"
            page.save(temp_image_path, "PNG")

            # Use OCR to extract text from the image of the page
            po_number = extract_po_number_from_image(temp_image_path)

            # Cleanup temporary file
            os.remove(temp_image_path)

            if po_number:
                return po_number

        logging.warning(f"No PO Number found in any page of the PDF: {pdf_path}")
        return None
    except Exception as e:
        logging.error(f"Error processing PDF {pdf_path}: {str(e)}")
        return None

def extract_po_number(file_path):
    """
    Determines the file type and extracts the Purchase Order (PO) number accordingly.
    Handles both PDFs and PNGs.
    """
    if file_path.lower().endswith('.png'):
        return extract_po_number_from_image(file_path)
    elif file_path.lower().endswith('.pdf'):
        return extract_po_number_from_pdf(file_path)
    else:
        logging.error(f"Unsupported file format for file: {file_path}")
        return None

if __name__ == "__main__":
    # Example usage
    logging.basicConfig(level=logging.INFO)
    file_path = "example_document.pdf"  # Replace with your file
    po_number = extract_po_number(file_path)
    if po_number:
        logging.info(f"Extracted PO Number: {po_number}")
    else:
        logging.info("PO Number could not be extracted.")
